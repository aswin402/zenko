import { memo, useEffect, useMemo, useRef, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import {
  ArrowUpRight,
  BrainIcon,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Clock,
  Edit3,
  FolderSearch,
  ListChecks,
  Search,
  type LucideIcon,
} from 'lucide-react'
import { motion } from 'motion/react'
import { EASE_OUT } from '@/lib/ease'
import { formatCompactTokens, formatTimeShort } from '@/lib/api'
import { extractCitationsFromTurn } from '@/lib/citations'
import {
  extractMemoryAndSkillChips,
  formatDayBucket,
  splitIntoStreamingSegments,
} from '@/lib/elements'
import type {
  ApprovalRecord,
  AskUserResolution,
  BrainstormSpec,
  ClarificationDecisionNote,
  ClarificationRecord,
  ComponentSlots,
  ExecutionPlan,
  OrchestratorStatus,
  PlanStepStatus,
  RewindMode,
  SessionEvent,
  SessionTask,
  SessionTaskStatus,
  UiCustomization,
  WorkspaceCheckpoint,
} from '@/lib/types'
import { safeHref } from '@/components/assistant-ui/utils/href'
import { cn } from '@/lib/utils'
import type { AgentActivityItem } from '@/components/agents/agent-activity'
import { MessageScroller } from '@/components/agents/message-scroller'
import { StreamingResponse } from '@/components/agents/streaming-response'
import { Stretchtext } from '@/components/aimran/stretchtext'
import {
  Alert as EaseAlert,
  AlertDescription as EaseAlertDescription,
  AlertTitle as EaseAlertTitle,
} from '@/components/easeui/alert'
import { MessageBubble as EaseMessageBubble } from '@/components/easeui/message-bubble'
import { StreamingResponse as EaseStreamingResponse } from '@/components/easeui/streaming-response'
import { StreamingResponse as RareStreamingResponse } from '@/components/rareui/streaming-response'
import {
  LineHoverLink,
  type LineHoverVariant,
} from '@/components/vengeanceui/line-hover-link'
import { AgentDisclosure } from '@/components/beui/AgentDisclosure'
import { AgentAvatar, UserAvatar } from '@/components/beui/ZenkoAvatars'
import {
  Alert,
  AlertAction,
  AlertContent,
  AlertDescription,
  AlertIcon,
  AlertTitle,
} from '@/components/motion/alert'
import type { LoaderVariant } from '@/components/motion/loader'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { CheckpointMenuButton } from '@/components/chat/Phase6Bar'
import { AskUserCard, AskUserReceipt } from '@/components/chat/AskUserCard'
import { DayDivider } from '@/components/assistant-ui/elements/day-separator'
import { StreamingText } from '@/components/assistant-ui/elements/streaming-text'
import { TodoList, type TodoStatus } from '@/components/assistant-ui/elements/todo-list'
import {
  ActivityDisplay,
  ApprovalElement,
  AssistantMessageFooter,
  CitationsDisplay,
  ErrorStateDisplay,
  LoadingStatusBar,
  MarkdownCodeBlock,
  MarkdownDataTable,
  SkillAndMemoryBadges,
  StoppedRunDisplay,
  type NormalizedToolStep,
} from '@/components/chat/ChatElements'

export const CHAT_WIDTH_CLASS: Record<ComponentSlots['chat_page']['content_width'], string> = {
  narrow: 'max-w-[680px]',
  balanced: 'max-w-[780px]',
  wide: 'max-w-[960px]',
  full: 'max-w-none',
}

export const USER_BUBBLE_CLASS: Record<ComponentSlots['chat_page']['user_bubble'], string> = {
  soft_card:
    'rounded-[15px] border border-[var(--border)] bg-[var(--card)] px-4 py-2.5 text-[var(--fg)]',
  easeui_bubble: '',
  accent_tint:
    'rounded-[15px] border border-[var(--accent)]/35 bg-[var(--accent)]/12 px-4 py-2.5 text-[var(--fg)]',
  solid_accent:
    'rounded-[15px] border border-transparent bg-[var(--accent)] px-4 py-2.5 text-[var(--primary-fg)]',
  minimal_border:
    'rounded-[14px] border border-[var(--border)] bg-transparent px-4 py-2.5 text-[var(--fg)]',
  background_only:
    'rounded-[15px] border border-transparent bg-[var(--card)] px-4 py-2.5 text-[var(--fg)]',
  borderless_minimal: 'border-0 bg-transparent px-1 py-1 text-[var(--fg)]',
  pill: 'rounded-3xl border border-[var(--border)] bg-[var(--card)] px-5 py-2.5 text-[var(--fg)]',
}

export const MODEL_BADGE_CLASS: Record<
  Exclude<NonNullable<ComponentSlots['chat_page']['model_badge_style']>, 'hidden'>,
  string
> = {
  bordered_chip:
    'rounded-md border border-[var(--border)] bg-[var(--card)] px-2 py-0.5 font-mono text-[11px] text-[var(--muted)]',
  border_only:
    'rounded-md border border-[var(--border)] bg-transparent px-2 py-0.5 font-mono text-[11px] text-[var(--muted)]',
  soft_fill:
    'rounded-md border border-transparent bg-[var(--card)] px-2 py-0.5 font-mono text-[11px] text-[var(--muted)]',
  accent_tint:
    'rounded-md border border-[var(--accent)]/35 bg-[color-mix(in_srgb,var(--accent)_11%,transparent)] px-2 py-0.5 font-mono text-[11px] text-[var(--accent)]',
  minimal_text: 'px-0.5 py-0.5 font-mono text-[11px] text-[var(--muted)]',
}

interface WelcomeStarter {
  id: string
  title: string
  subtitle: string
  prompt: string
  icon: LucideIcon
}

const WELCOME_STARTERS: readonly WelcomeStarter[] = [
  {
    id: 'workspace',
    title: 'Map Workspace',
    subtitle: 'Summarize architecture & crates',
    prompt: 'Inspect this workspace and summarize the architecture',
    icon: FolderSearch,
  },
  {
    id: 'memory',
    title: 'Memory & Skills',
    subtitle: 'Review active context & playbooks',
    prompt: 'Check memory snapshots and active skills',
    icon: BrainIcon,
  },
  {
    id: 'todos',
    title: 'Scan TODOs',
    subtitle: 'Find open tasks & tech debt',
    prompt: 'Search the codebase for TODOs',
    icon: Search,
  },
]

export interface LiveToolItem {
  call_id: string
  tool_name: string
  args_preview: string
  status: 'running' | 'ok' | 'error'
  duration_ms?: number
  diff?: string | null
  output_preview?: string
}

interface AssistantGroup {
  key: string
  model: string
  startedAt: string
  timestamp: string
  durationMs: number
  thinkingMs: number
  thinking: string
  text: string
  tools: {
    call_id?: string
    name: string
    args_preview?: string
    is_error: boolean
    duration_ms: number
    diff?: string | null
    content: string
  }[]
  plan?: ExecutionPlan | null
  brainstorms: BrainstormSpec[]
  decisions: ClarificationDecisionNote[]
  tokens: number
  cache_read: number
  cost_usd: number
}

function formatDurationSeconds(ms: number): string {
  const secs = Math.max(1, Math.round(ms / 1000))
  if (secs < 60) return `${secs}s`
  const mins = Math.floor(secs / 60)
  const rem = secs % 60
  return rem === 0 ? `${mins}m` : `${mins}m ${rem}s`
}

function computeGroupThoughtMs(group: AssistantGroup): number {
  if (group.thinkingMs > 0) return group.thinkingMs
  if (group.durationMs > 0) return group.durationMs
  const start = Date.parse(group.startedAt)
  const end = Date.parse(group.timestamp)
  if (Number.isFinite(start) && Number.isFinite(end) && end - start >= 500) {
    return end - start
  }
  const toolsMs = group.tools.reduce((acc, t) => acc + (t.duration_ms || 0), 0)
  if (toolsMs > 0) return toolsMs
  if (group.thinking.length > 0) {
    return Math.max(1000, Math.round((group.thinking.length / 180) * 1000))
  }
  return 0
}

const ARG_PREVIEW_PRIORITY_KEYS = [
  'path',
  'file_path',
  'file',
  'target',
  'command',
  'cmd',
  'action',
  'tool',
  'name',
  'id',
  'query',
  'pattern',
  'url',
  'cwd',
  'title',
  'reason',
] as const

const ARG_PREVIEW_PRIORITY_SET = new Set<string>(ARG_PREVIEW_PRIORITY_KEYS)

function compactArgsPreview(args: unknown): string {
  if (args == null) return ''
  if (typeof args === 'string') return args.length > 240 ? `${args.slice(0, 240)}…` : args
  if (typeof args === 'object' && !Array.isArray(args)) {
    const rec = args as Record<string, unknown>
    const orderedKeys: string[] = []
    for (const k of ARG_PREVIEW_PRIORITY_KEYS) {
      if (Object.prototype.hasOwnProperty.call(rec, k)) orderedKeys.push(k)
    }
    for (const k of Object.keys(rec)) {
      if (!ARG_PREVIEW_PRIORITY_SET.has(k)) orderedKeys.push(k)
    }
    const maxBudget = 240
    let out = '{'
    let written = 0
    for (const k of orderedKeys) {
      const v = rec[k]
      let valJson: string
      try {
        if (typeof v === 'string') {
          valJson = JSON.stringify(v.length > 120 ? `${v.slice(0, 120)}…` : v)
        } else {
          const raw = JSON.stringify(v)
          valJson = raw.length > 120 ? JSON.stringify(`${raw.slice(0, 100)}…`) : raw
        }
      } catch {
        continue
      }
      const entry = `${written > 0 ? ',' : ''}${JSON.stringify(k)}:${valJson}`
      if (out.length + entry.length + 1 > maxBudget) {
        if (written === 0) return `${`{${JSON.stringify(k)}:${valJson}}`.slice(0, maxBudget)}…`
        break
      }
      out += entry
      written += 1
    }
    return `${out}}`
  }
  try {
    const s = JSON.stringify(args)
    return s.length > 240 ? `${s.slice(0, 240)}…` : s
  } catch {
    return ''
  }
}

type RenderBlock =
  | { kind: 'user'; event: SessionEvent }
  | {
      kind: 'compaction_checkpoint'
      id: string
      compacted_events_count: number
      estimated_tokens_before: number
      estimated_tokens_after: number
      model: string
      summary: string
    }
  | { kind: 'assistant'; group: AssistantGroup }

const SYSTEM_NOTE_ATTACHERS: Record<
  string,
  (group: AssistantGroup, ev: SessionEvent) => void
> = {
  brainstorm_spec: (group, ev) => {
    if (ev.content_json?.spec) {
      group.brainstorms.push(ev.content_json.spec as BrainstormSpec)
    }
  },
  execution_plan: (group, ev) => {
    if (ev.content_json?.plan) {
      group.plan = ev.content_json.plan as ExecutionPlan
    }
  },
  clarification_decision: (group, ev) => {
    if (ev.content_json?.decision) {
      group.decisions.push(ev.content_json.decision as ClarificationDecisionNote)
    }
  },
}

function classifyActivityAction(toolName: string): 'read' | 'edit' | 'run' | string {
  const lower = toolName.toLowerCase()
  if (lower.includes('read') || lower.includes('list') || lower.includes('grep') || lower.includes('find')) {
    return 'read'
  }
  if (lower.includes('edit') || lower.includes('write') || lower.includes('patch') || lower.includes('create')) {
    return 'edit'
  }
  if (lower.includes('shell') || lower.includes('exec') || lower.includes('run') || lower.includes('test')) {
    return 'run'
  }
  return toolName
}

function countDiffDelta(diff?: string | null): { additions?: number; deletions?: number } {
  if (!diff) return {}
  let additions = 0
  let deletions = 0
  for (const line of diff.split('\n')) {
    if (line.startsWith('+') && !line.startsWith('+++')) additions += 1
    else if (line.startsWith('-') && !line.startsWith('---')) deletions += 1
  }
  return additions > 0 || deletions > 0 ? { additions, deletions } : {}
}

function groupEvents(events: SessionEvent[]): RenderBlock[] {
  const blocks: RenderBlock[] = []
  let current: AssistantGroup | null = null
  const toolArgsById = new Map<string, string>()

  const flush = () => {
    if (current) {
      blocks.push({ kind: 'assistant', group: current })
      current = null
    }
  }

  const ensureCurrent = (ev: SessionEvent, model = 'zenko'): AssistantGroup => {
    if (!current) {
      current = {
        key: ev.id,
        model,
        startedAt: ev.created_at,
        timestamp: ev.created_at,
        durationMs: 0,
        thinkingMs: 0,
        thinking: '',
        text: '',
        tools: [],
        plan: null,
        brainstorms: [],
        decisions: [],
        tokens: 0,
        cache_read: 0,
        cost_usd: 0,
      }
    }
    return current
  }

  for (const ev of events) {
    if (ev.role === 'system') {
      if (ev.kind === 'compaction_checkpoint') {
        flush()
        blocks.push({
          kind: 'compaction_checkpoint',
          id: ev.id,
          compacted_events_count: Number(ev.content_json?.compacted_events_count || 0),
          estimated_tokens_before: Number(ev.content_json?.estimated_tokens_before || 0),
          estimated_tokens_after: Number(ev.content_json?.estimated_tokens_after || 0),
          model: String(ev.content_json?.model || 'compactor'),
          summary: String(ev.content_json?.summary || ''),
        })
        continue
      }
      const attacher = SYSTEM_NOTE_ATTACHERS[String(ev.content_json?.note_kind || '')]
      if (attacher) {
        // Attach plans, brainstorms, and user decisions INSIDE the active assistant turn
        // instead of flushing and splitting one turn into detached chat blocks.
        attacher(ensureCurrent(ev), ev)
      }
      continue
    }
    if (ev.role === 'user') {
      flush()
      blocks.push({ kind: 'user', event: ev })
    } else if (ev.role === 'assistant') {
      const model = ev.content_json?.model || 'openrouter'
      const thinking = ev.content_json?.thinking || ''
      const text = ev.content_json?.text || ''
      const durationMs = Number(ev.content_json?.duration_ms || 0)
      const thinkingMs = Number(ev.content_json?.thinking_ms || 0)
      const rawCalls = Array.isArray(ev.content_json?.tool_calls) ? ev.content_json.tool_calls : []
      for (const tc of rawCalls) {
        if (tc && typeof tc.id === 'string') {
          toolArgsById.set(tc.id, compactArgsPreview(tc.arguments))
        }
      }
      if (!current) {
        current = {
          key: ev.id,
          model,
          startedAt: ev.created_at,
          timestamp: ev.created_at,
          durationMs,
          thinkingMs,
          thinking,
          text,
          tools: [],
          plan: null,
          brainstorms: [],
          decisions: [],
          tokens: ev.tokens_in + ev.tokens_out,
          cache_read: ev.cache_read_tokens,
          cost_usd: ev.cost_usd,
        }
      } else {
        if (model) current.model = model
        if (thinking) current.thinking = current.thinking ? `${current.thinking}\n\n${thinking}` : thinking
        if (text) current.text = current.text ? `${current.text}\n\n${text}` : text
        current.durationMs += durationMs
        current.thinkingMs += thinkingMs
        current.tokens += ev.tokens_in + ev.tokens_out
        current.cache_read += ev.cache_read_tokens
        current.cost_usd += ev.cost_usd
        current.timestamp = ev.created_at
      }
    } else if (ev.role === 'tool') {
      const grp = ensureCurrent(ev)
      const callId = typeof ev.content_json?.tool_call_id === 'string' ? ev.content_json.tool_call_id : undefined
      grp.tools.push({
        call_id: callId,
        name: ev.content_json?.tool_name || 'tool',
        args_preview: callId ? toolArgsById.get(callId) : undefined,
        is_error: Boolean(ev.content_json?.is_error),
        duration_ms: Number(ev.content_json?.duration_ms || 0),
        diff: ev.content_json?.diff || null,
        content: String(ev.content_json?.content || ''),
      })
      grp.timestamp = ev.created_at
    }
  }
  flush()
  return blocks
}

export const MarkdownProse = memo(function MarkdownProse({
  text,
  codeBlockStyle = 'aui_code_runner',
  tableStyle,
  linkHoverStyle = 'zenko_underline',
}: {
  text: string
  codeBlockStyle?: ComponentSlots['chat_page']['code_block_style']
  tableStyle?: UiCustomization['surface']['table_style']
  linkHoverStyle?: UiCustomization['surface']['link_hover_style']
}) {
  const markdownComponents = useMemo(
    () => ({
      pre({ children }: { children?: React.ReactNode }) {
        return <>{children}</>
      },
      code({ className, children }: { className?: string; children?: React.ReactNode }) {
        const isBlock = Boolean(className) || String(children).includes('\n')
        if (isBlock) {
          return (
            <MarkdownCodeBlock className={className} style={codeBlockStyle}>
              {children}
            </MarkdownCodeBlock>
          )
        }
        return <code>{children}</code>
      },
      table({ children }: { children?: React.ReactNode }) {
        return <MarkdownDataTable tableStyle={tableStyle}>{children}</MarkdownDataTable>
      },
      a({ href, children, ...props }: React.AnchorHTMLAttributes<HTMLAnchorElement>) {
        const validated = safeHref(typeof href === 'string' ? href : undefined)
        const isExternal = Boolean(
          validated && (validated.startsWith('http://') || validated.startsWith('https://')),
        )
        if (!validated) {
          return <span className="zenko-prose-link">{children}</span>
        }
        if (linkHoverStyle && linkHoverStyle.startsWith('skiper40_')) {
          const skiperVariant = linkHoverStyle.slice('skiper40_'.length)
          return (
            <a
              {...props}
              href={validated}
              target={isExternal ? '_blank' : undefined}
              rel={isExternal ? 'noopener noreferrer' : undefined}
              className={cn('skiper40-link', `skiper40-link--${skiperVariant}`)}
            >
              {children}
            </a>
          )
        }
        if (linkHoverStyle && linkHoverStyle !== 'zenko_underline') {
          return (
            <LineHoverLink
              {...props}
              variant={linkHoverStyle as LineHoverVariant}
              href={validated}
              target={isExternal ? '_blank' : undefined}
              rel={isExternal ? 'noopener noreferrer' : undefined}
            >
              {children}
            </LineHoverLink>
          )
        }
        return (
          <a
            {...props}
            href={validated}
            target={isExternal ? '_blank' : undefined}
            rel={isExternal ? 'noopener noreferrer' : undefined}
            className="zenko-prose-link"
          >
            {children}
            {isExternal && (
              <ArrowUpRight size={12} aria-hidden="true" className="zenko-prose-link-icon" />
            )}
          </a>
        )
      },
    }),
    [codeBlockStyle, tableStyle, linkHoverStyle],
  )

  return (
    <div className="zenko-prose" data-slot="message-content">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={markdownComponents}>
        {text}
      </ReactMarkdown>
    </div>
  )
})

const CompactionCheckpointPill = memo(function CompactionCheckpointPill({
  compacted_events_count,
  estimated_tokens_before,
  estimated_tokens_after,
  model,
  summary,
}: {
  compacted_events_count: number
  estimated_tokens_before: number
  estimated_tokens_after: number
  model: string
  summary: string
}) {
  const [open, setOpen] = useState(false)
  return (
    <div className="my-1 flex w-full flex-col items-center">
      <DayDivider
        onClick={() => setOpen((v) => !v)}
        label={
          <span>
            Auto-compacted · {compacted_events_count} events ·{' '}
            {formatCompactTokens(estimated_tokens_before)} →{' '}
            {formatCompactTokens(estimated_tokens_after)} tok · {model}
          </span>
        }
        trailing={
          <ChevronDown size={11.5} className={cn('transition-transform', open && 'rotate-180')} />
        }
      />
      {open && summary && (
        <div className="zenko-aui-paper mt-2 w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)] p-3.5 text-[12px] text-[var(--muted)]">
          <div className="text-[12px]">
            <MarkdownProse text={summary} />
          </div>
        </div>
      )}
    </div>
  )
})

const AssistantTurnView = memo(function AssistantTurnView({
  sessionId = null,
  group,
  fallbackTasks = [],
  ui,
  promptForRegen,
  parentForRegen,
  checkpoints = [],
  turnCheckpointId = null,
  showCheckpointCount = false,
  onRegenerate,
  onRetryTool,
  onRewindCheckpoint,
}: {
  sessionId?: string | null
  group: AssistantGroup
  fallbackTasks?: SessionTask[]
  ui: UiCustomization
  promptForRegen?: string
  parentForRegen?: string | null
  checkpoints?: WorkspaceCheckpoint[]
  turnCheckpointId?: string | null
  showCheckpointCount?: boolean
  onRegenerate?: (prompt: string, parentId?: string | null) => void
  onRetryTool?: (toolName: string, argsPreview?: string) => void
  onRewindCheckpoint?: (
    checkpointId: string,
    mode: RewindMode,
    overwriteManual: boolean,
  ) => Promise<void>
}) {
  const handleRegenerate = useMemo(
    () =>
      onRegenerate && promptForRegen
        ? () => onRegenerate(promptForRegen, parentForRegen)
        : undefined,
    [onRegenerate, promptForRegen, parentForRegen],
  )
  const [stepsOpen, setStepsOpen] = useState(false)
  const hasThinking = Boolean(ui.widgets.show_thinking && group.thinking)
  const hasInlinePlan = Boolean(group.plan) || fallbackTasks.length > 0
  const stepCount =
    group.tools.length +
    (hasThinking ? 1 : 0) +
    (hasInlinePlan ? 1 : 0) +
    group.brainstorms.length +
    group.decisions.length
  const thoughtMs = useMemo(() => computeGroupThoughtMs(group), [group])
  const thoughtSeconds = thoughtMs > 0 ? Math.max(1, Math.round(thoughtMs / 1000)) : 0
  const thoughtLabel = thoughtMs > 0 ? formatDurationSeconds(thoughtMs) : ''
  const { chips: badgeChips, skills: usedSkills } = useMemo(
    () => extractMemoryAndSkillChips(group.tools),
    [group.tools],
  )
  const shortModel = group.model.includes('/') ? group.model.split('/').pop() : group.model
  const chatSlot = ui.components.chat_page
  const skillBadgeStyle = chatSlot.skill_badge_style || 'aui_memory_chips'
  const actionsStyle = chatSlot.message_actions_style || 'aui_message_actions'
  const codeBlockStyle = chatSlot.code_block_style || 'aui_code_runner'

  const citations = useMemo(
    () => (chatSlot.show_citations ? extractCitationsFromTurn(group.text, group.tools) : []),
    [chatSlot.show_citations, group.text, group.tools],
  )

  const normalizedTools = useMemo<NormalizedToolStep[]>(
    () =>
      group.tools.map((t, idx) => ({
        id: t.call_id || `${group.key}-tool-${idx}`,
        name: t.name,
        args_preview: t.args_preview,
        status: t.is_error ? 'error' : 'ok',
        duration_ms: t.duration_ms,
        diff: t.diff,
        content: t.content,
      })),
    [group.key, group.tools],
  )

  const activityItems = useMemo<AgentActivityItem[]>(() => {
    const list: AgentActivityItem[] = []
    if (hasThinking) {
      list.push({
        id: `${group.key}-think`,
        type: 'trace',
        kind: 'thinking',
        label: thoughtLabel ? `Thought for ${thoughtLabel}` : 'Reasoning',
        detail: group.thinking.slice(0, 140).split('\n').join(' '),
      })
    }
    group.tools.forEach((t, idx) => {
      const delta = countDiffDelta(t.diff)
      list.push({
        id: `${group.key}-tool-${idx}`,
        type: 'tool',
        action: classifyActivityAction(t.name),
        target: `${t.name}${t.args_preview ? ` (${t.args_preview.slice(0, 48)})` : ''} · ${t.is_error ? 'err' : 'ok'} · ${t.duration_ms}ms`,
        ...delta,
      })
    })
    return list
  }, [group.key, group.thinking, group.tools, hasThinking, thoughtLabel])

  const useBeuiInlineActions =
    chatSlot.streaming_style === 'beui_stream' && actionsStyle === 'beui_inline'

  const checkpointActionNode =
    onRewindCheckpoint && checkpoints.length > 0 && (Boolean(turnCheckpointId) || showCheckpointCount) ? (
      <CheckpointMenuButton
        sessionId={sessionId}
        checkpoints={checkpoints}
        turnCheckpointId={turnCheckpointId}
        popoverStyle={ui.surface.popover_style || 'gooey'}
        showCount={showCheckpointCount && checkpoints.length > 1}
        minimalTrigger={actionsStyle === 'minimal' && !useBeuiInlineActions}
        onRewind={onRewindCheckpoint}
      />
    ) : null

  const tokenMetaNode =
    ui.widgets.show_cost_badges && group.tokens > 0 ? (
      <span className="font-mono text-[11px] tabular-nums text-[var(--muted)]/75">
        {group.tokens} tok
        {group.cache_read > 0 ? ` · ${group.cache_read} cached` : ''}
        {group.cost_usd > 0 ? ` · $${group.cost_usd.toFixed(4)}` : ''}
      </span>
    ) : undefined

  const completedSummaryText = useMemo(() => {
    const stepWord = `${stepCount} ${stepCount === 1 ? 'step' : 'steps'}`
    const todoTotal = fallbackTasks.length > 0 ? fallbackTasks.length : (group.plan?.steps.length ?? 0)
    const todoDone =
      fallbackTasks.length > 0
        ? fallbackTasks.filter((t) => t.status === 'done').length
        : (group.plan?.steps.filter((s) => s.status === 'completed').length ?? 0)
    const todoTag = todoTotal > 0 ? ` · ${todoDone}/${todoTotal} todos` : ''
    return `Completed ${stepWord}${todoTag}`
  }, [stepCount, fallbackTasks, group.plan])

  return (
    <motion.div
      data-slot="message"
      data-from="assistant"
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2, ease: EASE_OUT }}
      className="group relative flex items-start gap-3 sm:block"
    >
      {chatSlot.show_avatars && ui.motion.orb_style !== 'hidden' && (
        <div className="mt-0.5 shrink-0 sm:absolute sm:right-full sm:top-0.5 sm:mr-3 sm:mt-0">
          <AgentAvatar
            variant={ui.motion.orb_style}
            frame={chatSlot.agent_avatar_frame || 'squircle'}
            color={chatSlot.agent_avatar_color}
            icons={ui.icons}
            size={28}
          />
        </div>
      )}

      <div
        className={cn(
          'min-w-0 w-full flex-1',
          ui.layout.chat_layout === 'bubble_split' &&
            'rounded-2xl border border-[var(--border)] bg-[var(--surface)]/80 p-4',
        )}
      >
        {/* Meta row: Zenko + customizable model badge + quiet time */}
        <div className="mb-2 flex flex-wrap items-center gap-2 text-[12.5px]">
          <span className="font-medium text-[var(--fg)]/85">Zenko</span>
          {chatSlot.model_badge_style !== 'hidden' && (
            <span
              className={
                MODEL_BADGE_CLASS[chatSlot.model_badge_style || 'bordered_chip'] ||
                MODEL_BADGE_CLASS.bordered_chip
              }
            >
              {shortModel}
            </span>
          )}
          {ui.widgets.show_timestamps && (
            <span className="text-[11.5px] text-[var(--muted)]/75">
              {formatTimeShort(group.timestamp)}
            </span>
          )}
        </div>

        {/* Customizable Skill & Memory Badges (MemoryChips default | themed skill pills) */}
        <SkillAndMemoryBadges
          style={skillBadgeStyle}
          chips={badgeChips}
          skills={usedSkills}
          onToggleSteps={() => setStepsOpen((v) => !v)}
        />

        {/* Auto-Collapsed Completed Steps: Thought for Xs + Agent Activity + Smart Tool Elements */}
        {stepCount > 0 && chatSlot.thinking_style !== 'hidden' && (
          <div className="mb-3 flex flex-col gap-2">
            {chatSlot.thinking_style === 'aimran_stretchtext' ? (
              <div className="rounded-[var(--radius)] border border-[var(--border)]/75 bg-[var(--surface)]/65 px-3.5 py-2.5">
                <Stretchtext
                  size={13}
                  kicker={completedSummaryText}
                  mainColor="var(--accent)"
                  secondaryColor="color-mix(in srgb, var(--accent) 30%, var(--surface))"
                  toggleLabel="Expand all"
                  collapseLabel="Collapse"
                  segments={[
                    { kind: 'text', text: 'Zenko completed ' },
                    ...(hasThinking
                      ? [
                          {
                            kind: 'expand' as const,
                            id: 'reasoning',
                            trigger: thoughtLabel ? `reasoning (${thoughtLabel})` : 'reasoning',
                            detail: ` — ${group.thinking.slice(0, 360)}`,
                          },
                          ...(group.tools.length > 0
                            ? [{ kind: 'text' as const, text: ' and ' }]
                            : []),
                        ]
                      : []),
                    ...(group.tools.length > 0
                      ? [
                          {
                            kind: 'expand' as const,
                            id: 'tools',
                            trigger: `${group.tools.length} ${group.tools.length === 1 ? 'tool call' : 'tool calls'}`,
                            detail: ` (${group.tools.map((t) => `${t.name} ${t.duration_ms}ms`).join(', ')})`,
                          },
                        ]
                      : []),
                    { kind: 'text', text: '.' },
                  ]}
                />
              </div>
            ) : chatSlot.thinking_style === 'inline_quote' &&
              hasThinking &&
              group.tools.length === 0 &&
              !hasInlinePlan &&
              group.brainstorms.length === 0 &&
              group.decisions.length === 0 ? (
              <div className="border-l-2 border-[var(--accent)]/45 pl-3 font-mono text-[11.5px] whitespace-pre-wrap text-[var(--muted)]">
                {group.thinking}
              </div>
            ) : (
              <>
                <button
                  type="button"
                  aria-expanded={stepsOpen}
                  onClick={() => setStepsOpen((v) => !v)}
                  className={cn(
                    'group/steps inline-flex w-fit cursor-pointer items-center gap-2 rounded-lg text-[12.5px] text-[var(--muted)] transition-colors hover:text-[var(--fg)]',
                    chatSlot.thinking_style === 'minimal_bar'
                      ? 'rounded-full border border-[var(--border)] bg-[var(--surface)] px-2.5 py-0.5 font-mono text-[11px]'
                      : 'py-0.5',
                  )}
                >
                  <Clock size={12.5} className="shrink-0 opacity-75" />
                  <span className="font-medium tabular-nums">{completedSummaryText}</span>
                  <span className="font-mono text-[10.5px] text-[var(--muted)]/65 transition-colors group-hover/steps:text-[var(--accent)]">
                    {stepsOpen ? 'Hide' : 'Show'}
                  </span>
                  <ChevronDown
                    size={13.5}
                    className={cn('transition-transform duration-200', stepsOpen && 'rotate-180')}
                  />
                </button>

                <AgentDisclosure open={stepsOpen}>
                  <div className="mt-1 flex flex-col gap-3 border-l border-[var(--border)] pl-3.5">
                    {/* 1. Thought duration + full reasoning (borderless inside activity rail) */}
                    {hasThinking && (
                      <div className="flex flex-col gap-1">
                        <div className="flex items-center gap-1.5 font-mono text-[10.5px] uppercase tracking-wider text-[var(--muted)]">
                          <BrainIcon size={12} style={{ color: 'var(--accent)' }} />
                          <span>{thoughtLabel ? `Thought for ${thoughtLabel}` : 'Reasoning'}</span>
                        </div>
                        <CompactScrollArea autoHeight viewportClassName="max-h-56 pr-3">
                          <div className="font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-[var(--muted)]">
                            {group.thinking}
                          </div>
                        </CompactScrollArea>
                      </div>
                    )}

                    {/* 2. Plan & Todos inside Agent Activity (borderless, zero detached card) */}
                    {hasInlinePlan && (
                      <InlinePlanActivity plan={group.plan ?? null} tasks={fallbackTasks} />
                    )}

                    {/* 3. Brainstorm Specs inside Agent Activity (borderless) */}
                    {group.brainstorms.map((spec) => (
                      <InlineBrainstormActivity key={spec.id} spec={spec} />
                    ))}

                    {/* 4. Resolved User Input Receipts inside Agent Activity (borderless) */}
                    {group.decisions.length > 0 && (
                      <div className="flex flex-col gap-2">
                        <span className="font-mono text-[10.5px] uppercase tracking-wider text-[var(--muted)]">
                          User Input ({group.decisions.length})
                        </span>
                        {group.decisions.map((dec, idx) => (
                          <AskUserReceipt key={`${group.key}-dec-${idx}`} decision={dec} />
                        ))}
                      </div>
                    )}

                    {/* 5. Unified Agent Activity & Tool Execution (zero duplicate tool lists) */}
                    {chatSlot.activity_style !== 'hidden' &&
                      (activityItems.length > 0 || normalizedTools.length > 0) && (
                        <div className="flex flex-col gap-1.5">
                          <span className="font-mono text-[10.5px] uppercase tracking-wider text-[var(--muted)]">
                            Agent Activity
                            {normalizedTools.length > 0 ? ` (${normalizedTools.length})` : ''}
                          </span>
                          <ActivityDisplay
                            style={chatSlot.activity_style}
                            items={activityItems}
                            tools={normalizedTools}
                            status="complete"
                            statusLabel={completedSummaryText}
                            durationSeconds={thoughtSeconds}
                            durationLabel={thoughtLabel || undefined}
                            defaultOpen={true}
                            diffStyle={chatSlot.diff_style}
                            shellStyle={chatSlot.shell_style}
                            fileTreeStyle={chatSlot.file_tree_style}
                            webSearchStyle={chatSlot.web_search_style}
                            onRetryTool={onRetryTool}
                          />
                        </div>
                      )}
                  </div>
                </AgentDisclosure>
              </>
            )}
          </div>
        )}

        {/* Response Body + Message Actions (Copy + Retry + Citations + tok · cached) */}
        {group.text &&
          (chatSlot.streaming_style === 'easeui_stream' ? (
            <>
              <EaseStreamingResponse
                status="complete"
                copyText={group.text}
                onRetry={handleRegenerate}
                showFeedback
              >
                <MarkdownProse
                  text={group.text}
                  codeBlockStyle={codeBlockStyle}
                  tableStyle={ui.surface.table_style}
                  linkHoverStyle={ui.surface.link_hover_style}
                />
              </EaseStreamingResponse>
              {(citations.length > 0 || checkpointActionNode || tokenMetaNode) && (
                <div className="mt-1.5 flex flex-wrap items-center gap-2.5">
                  {checkpointActionNode}
                  <CitationsDisplay citations={citations} style={chatSlot.citations_style} />
                  {tokenMetaNode}
                </div>
              )}
            </>
          ) : chatSlot.streaming_style === 'rareui_stream' ? (
            <>
              <RareStreamingResponse
                text={group.text}
                status="done"
                ink="var(--accent)"
                actions={true}
                linkHoverStyle={ui.surface.link_hover_style}
                onRetry={handleRegenerate}
              />
              {(citations.length > 0 || checkpointActionNode || tokenMetaNode) && (
                <div className="mt-1.5 flex flex-wrap items-center gap-2.5">
                  {checkpointActionNode}
                  <CitationsDisplay citations={citations} style={chatSlot.citations_style} />
                  {tokenMetaNode}
                </div>
              )}
            </>
          ) : useBeuiInlineActions ? (
            <>
              <StreamingResponse
                status="complete"
                copyText={group.text}
                onRetry={handleRegenerate}
                extraActions={checkpointActionNode}
                sources={
                  !chatSlot.citations_style || chatSlot.citations_style === 'beui_citations'
                    ? citations
                    : []
                }
                announce={false}
                contentClassName="text-[0.94rem]"
                footerMeta={tokenMetaNode}
              >
                <MarkdownProse
                  text={group.text}
                  codeBlockStyle={codeBlockStyle}
                  tableStyle={ui.surface.table_style}
                  linkHoverStyle={ui.surface.link_hover_style}
                />
              </StreamingResponse>
              {citations.length > 0 &&
                chatSlot.citations_style &&
                chatSlot.citations_style !== 'beui_citations' && (
                  <div className="mt-1.5">
                    <CitationsDisplay citations={citations} style={chatSlot.citations_style} />
                  </div>
                )}
            </>
          ) : (
            <>
              <MarkdownProse
                text={group.text}
                codeBlockStyle={codeBlockStyle}
                tableStyle={ui.surface.table_style}
                linkHoverStyle={ui.surface.link_hover_style}
              />
              <AssistantMessageFooter
                style={actionsStyle}
                citationsStyle={chatSlot.citations_style}
                text={group.text}
                citations={citations}
                extraActions={checkpointActionNode}
                tokenMeta={tokenMetaNode}
                onRegenerate={handleRegenerate}
                onToggleSteps={stepCount > 0 ? () => setStepsOpen((v) => !v) : undefined}
              />
            </>
          ))}
      </div>
    </motion.div>
  )
})

function InlineBrainstormActivity({ spec }: { spec: BrainstormSpec }) {
  const stageLabel = spec.stage || (spec.selected_option_id ? 'committed' : 'exploring')
  return (
    <div className="flex flex-col gap-2 text-[12px]">
      <div className="flex flex-wrap items-center gap-1.5 font-mono text-[10.5px] uppercase tracking-wider text-[var(--muted)]">
        <BrainIcon size={12} style={{ color: 'var(--accent)' }} />
        <span>Brainstorm</span>
        <span className="text-[var(--accent)]">· {stageLabel}</span>
        <span>· {spec.options.length} options</span>
      </div>
      <p className="text-[12.5px] leading-relaxed text-[var(--fg)]/85">{spec.problem}</p>
      {spec.options.length > 0 && (
        <div className="flex flex-col gap-1.5">
          {spec.options.map((opt) => {
            const isSelected = Boolean(spec.selected_option_id) && opt.id === spec.selected_option_id
            return (
              <div
                key={opt.id}
                className={cn(
                  'border-l pl-2.5 py-0.5',
                  isSelected ? 'border-[var(--accent)]' : 'border-[var(--border)]/70',
                )}
              >
                <div className="flex items-center gap-2">
                  <span className="font-medium text-[var(--fg)]/90">{opt.title}</span>
                  {isSelected && (
                    <span className="font-mono text-[10px] uppercase text-[var(--accent)]">
                      selected
                    </span>
                  )}
                  <span className="font-mono text-[10px] text-[var(--muted)]">
                    rev:{opt.reversibility} · blast:{opt.blast_radius} · safe:{opt.safety_score}/10
                  </span>
                </div>
                {opt.summary && <p className="text-[11.5px] text-[var(--muted)]">{opt.summary}</p>}
              </div>
            )
          })}
        </div>
      )}
      {spec.selection_rationale && (
        <p className="text-[11.5px] text-[var(--muted)]">
          <span className="font-medium text-[var(--fg)]/80">Rationale:</span> {spec.selection_rationale}
        </p>
      )}
      {spec.verification_plan.length > 0 && (
        <div className="font-mono text-[10.5px] text-[var(--muted)]">
          Verify: {spec.verification_plan.join(' · ')}
        </div>
      )}
    </div>
  )
}

const PLAN_STATUS_TO_TODO: Record<PlanStepStatus, TodoStatus> = {
  pending: 'pending',
  in_progress: 'active',
  completed: 'done',
  blocked: 'cancelled',
}

const TASK_STATUS_TO_TODO: Record<SessionTaskStatus, TodoStatus> = {
  pending: 'pending',
  in_progress: 'active',
  blocked: 'cancelled',
  done: 'done',
  failed: 'failed',
}

function InlinePlanActivity({
  plan,
  tasks = [],
}: {
  plan: ExecutionPlan | null
  tasks?: SessionTask[]
}) {
  const todoItems = useMemo(() => {
    if (tasks.length > 0) {
      return tasks.map((t) => ({
        id: t.id,
        text: t.title,
        status: TASK_STATUS_TO_TODO[t.status] || 'pending',
        description:
          t.active_label ||
          (t.owner_agent
            ? `owner: ${t.owner_agent.slice(0, 12)}`
            : t.verification_cmd
              ? `verify: ${t.verification_cmd}`
              : undefined),
        reason: t.status === 'failed' ? t.notes || t.verification_evidence || undefined : undefined,
      }))
    }
    if (!plan) return []
    return plan.steps.map((step) => ({
      id: step.id,
      text: step.title,
      status: PLAN_STATUS_TO_TODO[step.status] || 'pending',
      description:
        step.depends_on.length > 0
          ? `after: ${step.depends_on.join(', ')}${step.verification ? ` · verify: ${step.verification}` : ''}`
          : step.verification
            ? `verify: ${step.verification}`
            : undefined,
    }))
  }, [plan, tasks])

  if (todoItems.length === 0) return null

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center gap-1.5 font-mono text-[10.5px] uppercase tracking-wider text-[var(--muted)]">
        <ListChecks size={12} style={{ color: 'var(--accent)' }} />
        <span>Plan &amp; Todos</span>
      </div>
      <TodoList
        title={plan?.goal || 'Tracked Plan'}
        className="max-w-none"
        items={todoItems}
      />
    </div>
  )
}

function LiveAssistantTurn({
  persistedGroup,
  sessionTasks = [],
  liveOrchestrator,
  liveThinking,
  liveText,
  liveTools,
  isRunning,
  turnStartedAt,
  activeModel,
  ui,
}: {
  persistedGroup: AssistantGroup | null
  sessionTasks?: SessionTask[]
  liveOrchestrator: OrchestratorStatus | null
  liveThinking: string
  liveText: string
  liveTools: LiveToolItem[]
  isRunning: boolean
  turnStartedAt: number | null
  activeModel: string
  ui: UiCustomization
}) {
  const [liveOpen, setLiveOpen] = useState(true)
  const mountMsRef = useRef<number>(Date.now())
  const [nowMs, setNowMs] = useState<number>(() => Date.now())

  useEffect(() => {
    if (!isRunning) return
    setNowMs(Date.now())
    const id = window.setInterval(() => setNowMs(Date.now()), 1000)
    return () => window.clearInterval(id)
  }, [isRunning])

  const chatSlot = ui.components.chat_page
  const loaderVariant = (chatSlot.loader_variant || 'dot-matrix') as LoaderVariant

  const effectiveModel = liveOrchestrator?.model || persistedGroup?.model || activeModel
  const shortModel = effectiveModel.includes('/')
    ? effectiveModel.split('/').pop()
    : effectiveModel

  const mergedThinking = useMemo(() => {
    const a = persistedGroup?.thinking?.trim() || ''
    const b = liveThinking.trim()
    if (!a) return liveThinking
    if (!b) return persistedGroup?.thinking || ''
    if (a.endsWith(b)) return persistedGroup?.thinking || ''
    return `${persistedGroup?.thinking}\n\n${liveThinking}`
  }, [persistedGroup?.thinking, liveThinking])

  const mergedText = useMemo(() => {
    const a = persistedGroup?.text?.trim() || ''
    const b = liveText.trim()
    if (!a) return liveText
    if (!b) return persistedGroup?.text || ''
    if (a.endsWith(b)) return persistedGroup?.text || ''
    return `${persistedGroup?.text}\n\n${liveText}`
  }, [persistedGroup?.text, liveText])

  const mergedTools = useMemo<LiveToolItem[]>(() => {
    const out: LiveToolItem[] = []
    const seen = new Set<string>()
    if (persistedGroup) {
      persistedGroup.tools.forEach((t, idx) => {
        const id = t.call_id || `persisted-${persistedGroup.key}-${idx}`
        seen.add(id)
        out.push({
          call_id: id,
          tool_name: t.name,
          args_preview: t.args_preview || '',
          status: t.is_error ? 'error' : 'ok',
          duration_ms: t.duration_ms,
          diff: t.diff,
          output_preview: t.content,
        })
      })
    }
    for (const t of liveTools) {
      if (t.call_id && seen.has(t.call_id)) continue
      if (t.call_id) seen.add(t.call_id)
      out.push(t)
    }
    return out
  }, [persistedGroup, liveTools])

  const startedMs = useMemo(() => {
    if (persistedGroup?.startedAt) {
      const parsed = Date.parse(persistedGroup.startedAt)
      if (Number.isFinite(parsed)) return parsed
    }
    return turnStartedAt ?? mountMsRef.current
  }, [persistedGroup?.startedAt, turnStartedAt])

  const elapsedMs = Math.max(1000, nowMs - startedMs)
  const elapsedLabel = formatDurationSeconds(elapsedMs)

  const liveActivityItems = useMemo<AgentActivityItem[]>(() => {
    const items: AgentActivityItem[] = []
    if (liveOrchestrator) {
      items.push({
        id: 'live-orch',
        type: 'step',
        status: 'active',
        label: `${liveOrchestrator.tier} · ${liveOrchestrator.stage}`,
        meta: liveOrchestrator.detail || undefined,
      })
    }
    for (const t of mergedTools) {
      const delta = countDiffDelta(t.diff)
      items.push({
        id: t.call_id,
        type: 'tool',
        action: classifyActivityAction(t.tool_name),
        target: `${t.tool_name}${t.args_preview ? ` (${t.args_preview.slice(0, 56)})` : ''}${
          t.status === 'running'
            ? ' · running'
            : t.duration_ms !== undefined
              ? ` · ${t.duration_ms}ms`
              : ''
        }`,
        ...delta,
      })
    }
    return items
  }, [liveOrchestrator, mergedTools])

  const runningTool = mergedTools.find((t) => t.status === 'running')
  const liveTodoTag = useMemo(() => {
    const total = sessionTasks.length > 0 ? sessionTasks.length : (persistedGroup?.plan?.steps.length ?? 0)
    if (total === 0) return ''
    const done =
      sessionTasks.length > 0
        ? sessionTasks.filter((t) => t.status === 'done').length
        : (persistedGroup?.plan?.steps.filter((s) => s.status === 'completed').length ?? 0)
    return ` · ${done}/${total} todos`
  }, [sessionTasks, persistedGroup?.plan])
  const liveStatusText = useMemo(() => {
    if (runningTool) return `Running ${runningTool.tool_name}...${liveTodoTag}`
    if (liveText) {
      return `Writing response · ${mergedTools.length} ${mergedTools.length === 1 ? 'tool' : 'tools'}${liveTodoTag}`
    }
    if (mergedThinking) return `Thinking...${liveTodoTag}`
    if (liveOrchestrator?.stage) return `${liveOrchestrator.stage}...${liveTodoTag}`
    return `Thinking...${liveTodoTag}`
  }, [runningTool, liveText, mergedTools.length, mergedThinking, liveOrchestrator?.stage, liveTodoTag])

  const hasLivePlan = Boolean(persistedGroup?.plan) || sessionTasks.length > 0
  const liveBrainstorms = persistedGroup?.brainstorms || []
  const liveDecisions = persistedGroup?.decisions || []
  const hasWorkDetails =
    Boolean(ui.widgets.show_thinking && mergedThinking) ||
    hasLivePlan ||
    liveBrainstorms.length > 0 ||
    liveDecisions.length > 0 ||
    mergedTools.length > 0 ||
    liveActivityItems.length > 0

  return (
    <div data-slot="message" data-from="assistant" className="relative flex items-start gap-3 sm:block">
      {chatSlot.show_avatars && ui.motion.orb_style !== 'hidden' && (
        <div className="mt-0.5 shrink-0 sm:absolute sm:right-full sm:top-0.5 sm:mr-3 sm:mt-0">
          <AgentAvatar
            variant={ui.motion.orb_style}
            frame={chatSlot.agent_avatar_frame || 'squircle'}
            color={chatSlot.agent_avatar_color}
            icons={ui.icons}
            size={28}
            status="running"
          />
        </div>
      )}
      <div className="min-w-0 w-full flex-1">
        {/* Single Unified Zenko Header */}
        <div className="mb-2 flex flex-wrap items-center gap-2 text-[12.5px]">
          <span className="font-medium text-[var(--fg)]/85">Zenko</span>
          {chatSlot.model_badge_style !== 'hidden' && (
            <span
              className={
                MODEL_BADGE_CLASS[chatSlot.model_badge_style || 'bordered_chip'] ||
                MODEL_BADGE_CLASS.bordered_chip
              }
            >
              {shortModel}
            </span>
          )}
          {liveOrchestrator && (
            <span className="rounded-md border border-[var(--border)] bg-[var(--surface)] px-2 py-0.5 font-mono text-[10.5px] text-[var(--muted)]">
              {liveOrchestrator.tier} · {liveOrchestrator.stage}
            </span>
          )}
        </div>

        {/* Live Animated Thinking + Streaming Words + Tool Runs & Agent Activity */}
        {chatSlot.thinking_style !== 'hidden' && (
          <div className="mb-3 flex flex-col gap-2">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <button
                type="button"
                aria-expanded={liveOpen}
                onClick={() => hasWorkDetails && setLiveOpen((v) => !v)}
                className={cn(
                  'inline-flex items-center gap-2.5 text-[13px]',
                  hasWorkDetails && 'cursor-pointer',
                )}
              >
                <LoadingStatusBar
                  loadingStyle={chatSlot.loading_style}
                  loaderVariant={loaderVariant}
                  label={liveStatusText}
                  elapsedLabel={elapsedLabel}
                  active={isRunning}
                />
                {hasWorkDetails && (
                  <span className="inline-flex items-center gap-1 font-mono text-[10.5px] text-[var(--muted)]/70 hover:text-[var(--fg)]">
                    <span>{liveOpen ? 'Hide' : 'Show'}</span>
                    <ChevronDown
                      size={12.5}
                      className={cn('transition-transform duration-200', liveOpen && 'rotate-180')}
                    />
                  </span>
                )}
              </button>
            </div>

            {hasWorkDetails && (
              <AgentDisclosure open={liveOpen}>
                <div className="flex flex-col gap-3 border-l border-[var(--border)] pl-3.5">
                  {/* 1. Live streaming reasoning (borderless inside activity rail) */}
                  {ui.widgets.show_thinking && mergedThinking && (
                    <div className="flex flex-col gap-1">
                      <div className="flex items-center gap-1.5 font-mono text-[10.5px] uppercase tracking-wider text-[var(--muted)]">
                        <BrainIcon size={12} style={{ color: 'var(--accent)' }} />
                        <span>Thinking · {elapsedLabel}</span>
                      </div>
                      <CompactScrollArea autoHeight stickToBottom viewportClassName="max-h-40 pr-3">
                        <div className="font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-[var(--muted)]">
                          {mergedThinking}
                          {isRunning && !liveText && !runningTool && (
                            <span
                              aria-hidden="true"
                              className="ml-0.5 inline-block h-3 w-1.5 animate-pulse rounded-[1px] align-middle"
                              style={{ backgroundColor: 'var(--accent)' }}
                            />
                          )}
                        </div>
                      </CompactScrollArea>
                    </div>
                  )}

                  {/* 2. Live Plan & Todos inside Agent Activity (borderless) */}
                  {hasLivePlan && (
                    <InlinePlanActivity
                      plan={persistedGroup?.plan ?? null}
                      tasks={sessionTasks}
                    />
                  )}

                  {/* 3. Live Brainstorms & User Decisions inside Agent Activity */}
                  {liveBrainstorms.map((spec) => (
                    <InlineBrainstormActivity key={spec.id} spec={spec} />
                  ))}
                  {liveDecisions.length > 0 && (
                    <div className="flex flex-col gap-2">
                      <span className="font-mono text-[10.5px] uppercase tracking-wider text-[var(--muted)]">
                        User Input ({liveDecisions.length})
                      </span>
                      {liveDecisions.map((dec, idx) => (
                        <AskUserReceipt key={`live-dec-${idx}`} decision={dec} />
                      ))}
                    </div>
                  )}

                  {/* 4. Unified Live Agent Activity & Tool Execution */}
                  {chatSlot.activity_style !== 'hidden' &&
                    (liveActivityItems.length > 0 || mergedTools.length > 0) && (
                      <div className="flex flex-col gap-1.5">
                        <span className="font-mono text-[10.5px] uppercase tracking-wider text-[var(--muted)]">
                          Agent Activity
                          {mergedTools.length > 0 ? ` (${mergedTools.length})` : ''}
                        </span>
                        <ActivityDisplay
                          style={chatSlot.activity_style}
                          items={liveActivityItems}
                          tools={mergedTools.map((t) => ({
                            id: t.call_id,
                            name: t.tool_name,
                            args_preview: t.args_preview,
                            status: t.status,
                            duration_ms: t.duration_ms,
                            diff: t.diff,
                            content: t.output_preview || '',
                          }))}
                          status={isRunning && !liveText ? 'working' : 'complete'}
                          statusLabel={liveStatusText}
                          durationLabel={elapsedLabel}
                          defaultOpen={true}
                          diffStyle={chatSlot.diff_style}
                          shellStyle={chatSlot.shell_style}
                          fileTreeStyle={chatSlot.file_tree_style}
                          webSearchStyle={chatSlot.web_search_style}
                        />
                      </div>
                    )}
                </div>
              </AgentDisclosure>
            )}
          </div>
        )}

        {/* Live Streaming Response Words (StreamingText | BeUI | EaseUI | RareUI | Editorial) */}
        {mergedText &&
          (chatSlot.streaming_style === 'aui_streaming_text' ? (
            (() => {
              const { segments, wordCount } = splitIntoStreamingSegments(mergedText)
              return (
                <StreamingText
                  segments={segments}
                  count={wordCount}
                  streaming={isRunning}
                />
              )
            })()
          ) : chatSlot.streaming_style === 'easeui_stream' ? (
            <EaseStreamingResponse status={isRunning ? 'streaming' : 'complete'}>
              <MarkdownProse
                text={mergedText}
                codeBlockStyle={chatSlot.code_block_style || 'aui_code_runner'}
                tableStyle={ui.surface.table_style}
                linkHoverStyle={ui.surface.link_hover_style}
              />
            </EaseStreamingResponse>
          ) : chatSlot.streaming_style === 'rareui_stream' ? (
            <RareStreamingResponse
              text={mergedText}
              status={isRunning ? 'streaming' : 'done'}
              ink="var(--accent)"
              actions={false}
              linkHoverStyle={ui.surface.link_hover_style}
            />
          ) : chatSlot.streaming_style === 'beui_stream' ? (
            <StreamingResponse status="streaming" announce={false} showActions={false}>
              <MarkdownProse
                text={mergedText}
                codeBlockStyle={chatSlot.code_block_style || 'aui_code_runner'}
                tableStyle={ui.surface.table_style}
                linkHoverStyle={ui.surface.link_hover_style}
              />
            </StreamingResponse>
          ) : (
            <MarkdownProse
              text={mergedText}
              codeBlockStyle={chatSlot.code_block_style || 'aui_code_runner'}
              tableStyle={ui.surface.table_style}
              linkHoverStyle={ui.surface.link_hover_style}
            />
          ))}
      </div>
    </div>
  )
}

export function ChatStream({
  sessionId = null,
  events,
  siblings,
  checkpoints = [],
  sessionTasks = [],
  pendingApprovals,
  pendingClarifications = [],
  liveOrchestrator = null,
  liveThinking,
  liveText,
  liveTools,
  isRunning,
  turnStartedAt = null,
  liveTurnError = null,
  stoppedRun = null,
  activeModel,
  ui,
  setupCompleted,
  onQuickPrompt,
  onRetryTurn,
  onContinueStopped,
  onDiscardStopped,
  onRegenerate,
  onRetryTool,
  onRewindCheckpoint,
  onOpenSetup,
  onEditBranch,
  onSwitchBranch,
  onResolveApproval,
  onResolveClarification,
}: {
  sessionId?: string | null
  events: SessionEvent[]
  siblings: Record<string, string[]>
  checkpoints?: WorkspaceCheckpoint[]
  sessionTasks?: SessionTask[]
  pendingApprovals: ApprovalRecord[]
  pendingClarifications?: ClarificationRecord[]
  liveOrchestrator?: OrchestratorStatus | null
  liveThinking: string
  liveText: string
  liveTools: LiveToolItem[]
  isRunning: boolean
  turnStartedAt?: number | null
  liveTurnError?: { title: string; detail: string; retrying: boolean } | null
  stoppedRun?: { words: string[]; reason: string } | null
  activeModel: string
  ui: UiCustomization
  setupCompleted: boolean
  onQuickPrompt: (p: string) => void
  onRetryTurn?: () => void
  onContinueStopped?: () => void
  onDiscardStopped?: () => void
  onRegenerate?: (promptText: string, parentId?: string | null) => void
  onRetryTool?: (toolName: string, argsPreview?: string) => void
  onRewindCheckpoint?: (
    checkpointId: string,
    mode: RewindMode,
    overwriteManual: boolean,
  ) => Promise<void>
  onOpenSetup: () => void
  onEditBranch: (parentId: string | null, text: string) => void
  onSwitchBranch: (eventId: string) => void
  onResolveApproval: (id: string, decision: 'approve' | 'deny' | 'always_allow') => void
  onResolveClarification?: (id: string, resolution: AskUserResolution) => void | Promise<void>
}) {
  const bottomRef = useRef<HTMLDivElement>(null)
  const blocks = useMemo(() => groupEvents(events), [events])
  const chatSlot = ui.components.chat_page
  const scrollerStyle = chatSlot.scroller_style || 'beui_rail'

  // Unified single-turn merge: if the session is actively running and earlier iterations
  // of THIS turn were already persisted into SQLite, merge that trailing assistant block
  // into LiveAssistantTurn so we NEVER render duplicate Zenko headers for one prompt —
  // while guarding against stealing a previous completed turn before the new user prompt lands!
  const hasLiveTurn = isRunning || Boolean(liveText || liveThinking || liveTools.length > 0)
  const lastBlock = blocks[blocks.length - 1]
  const lastAssistantStartedMs =
    lastBlock?.kind === 'assistant' && lastBlock.group.startedAt
      ? Date.parse(lastBlock.group.startedAt)
      : NaN
  const belongsToCurrentTurn =
    !turnStartedAt ||
    !Number.isFinite(lastAssistantStartedMs) ||
    lastAssistantStartedMs >= turnStartedAt - 1500
  const mergeLastIntoLive =
    hasLiveTurn && lastBlock?.kind === 'assistant' && belongsToCurrentTurn
  const historicalBlocks = mergeLastIntoLive ? blocks.slice(0, -1) : blocks
  const activePersistedGroup = mergeLastIntoLive ? lastBlock.group : null

  // Only manual-scroll when using CompactScrollArea fallback (BeUI MessageScroller manages followOutput natively!)
  useEffect(() => {
    if (scrollerStyle === 'beui_rail' || scrollerStyle === 'beui_scroller') return
    bottomRef.current?.scrollIntoView({ behavior: 'smooth', block: 'end' })
  }, [scrollerStyle, events.length, liveText, liveThinking, liveTools.length, pendingApprovals.length])

  const widthClass = CHAT_WIDTH_CLASS[chatSlot.content_width] || CHAT_WIDTH_CLASS.balanced
  const userBubbleClass = USER_BUBBLE_CLASS[chatSlot.user_bubble] || USER_BUBBLE_CLASS.soft_card

  const streamContent = (
    <div
      className={cn(
        'mx-auto flex w-full flex-col gap-7',
        widthClass,
        ui.layout.chat_layout === 'terminal_stream' && 'font-mono',
      )}
    >
      {blocks.length === 0 && !isRunning && (
        <motion.div
          initial={{ opacity: 0, y: 10 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.28, ease: EASE_OUT }}
          className="mx-auto flex w-full max-w-[660px] flex-col items-center py-20 text-center"
        >
          <div className="relative mb-6 flex items-center justify-center">
            <div
              aria-hidden="true"
              className="pointer-events-none absolute h-28 w-28 rounded-full blur-2xl"
              style={{
                background: 'radial-gradient(circle, color-mix(in srgb, var(--accent) 26%, transparent) 0%, transparent 72%)',
              }}
            />
            <AgentAvatar
              variant={ui.motion.orb_style === 'hidden' ? 'liquid_halo' : ui.motion.orb_style}
              frame={chatSlot.agent_avatar_frame || 'squircle'}
              color={chatSlot.agent_avatar_color}
              icons={ui.icons}
              size={64}
              className={cn(
                'relative',
                (chatSlot.agent_avatar_frame || 'squircle') !== 'bare' &&
                  'shadow-[0_10px_30px_rgba(0,0,0,0.32)]',
              )}
            />
          </div>

          <h1
            className="mb-2.5 text-[26px] font-semibold tracking-[-0.035em] text-[var(--fg)] sm:text-[28px]"
            style={{ fontFamily: 'var(--font-display)' }}
          >
            What should Zenko work on?
          </h1>
          <p className="mb-8 max-w-xl text-[13.5px] text-[var(--muted)]">
            Autonomous local-first engineering — always on, always yours.
          </p>

          {!setupCompleted && (
            <div className="mb-7 w-full max-w-md text-left">
              {ui.surface.alert_style === 'easeui_alert' ? (
                <EaseAlert variant="warning">
                  <div className="flex flex-1 flex-col gap-2">
                    <div>
                      <EaseAlertTitle>Connect a provider to get started</EaseAlertTitle>
                      <EaseAlertDescription>
                        Add your OpenRouter, MiniMax, Ollama, or Telegram credentials in Setup.
                      </EaseAlertDescription>
                    </div>
                    <button
                      type="button"
                      onClick={onOpenSetup}
                      className="z-btn z-btn-primary z-btn-sm w-fit"
                    >
                      Configure Providers &amp; Telegram
                    </button>
                  </div>
                </EaseAlert>
              ) : ui.surface.alert_style !== 'compact' ? (
                <Alert variant="info">
                  <AlertIcon />
                  <AlertContent>
                    <AlertTitle>Connect a provider to get started</AlertTitle>
                    <AlertDescription>
                      Add your OpenRouter, MiniMax, Ollama, or Telegram credentials in Setup.
                    </AlertDescription>
                    <AlertAction>
                      <button
                        type="button"
                        onClick={onOpenSetup}
                        className="z-btn z-btn-primary z-btn-sm"
                      >
                        Configure Providers & Telegram
                      </button>
                    </AlertAction>
                  </AlertContent>
                </Alert>
              ) : (
                <button
                  type="button"
                  onClick={onOpenSetup}
                  className="z-btn z-btn-primary w-full px-4 py-2.5 text-[13px]"
                >
                  Configure OpenRouter & Telegram
                </button>
              )}
            </div>
          )}

          <div className="grid w-full grid-cols-1 gap-2.5 text-left sm:grid-cols-3">
            {WELCOME_STARTERS.map((starter) => {
              const IconComp = starter.icon
              return (
                <button
                  key={starter.id}
                  type="button"
                  onClick={() => onQuickPrompt(starter.prompt)}
                  className="group flex cursor-pointer flex-col gap-2 rounded-xl border-t border-[var(--border)]/90 bg-[var(--surface)]/55 px-4 py-3.5 text-left transition-all duration-150 hover:-translate-y-0.5 hover:border-[var(--accent)]/65 hover:bg-[var(--card)]/85"
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="flex h-7 w-7 items-center justify-center rounded-lg bg-[var(--card)] text-[var(--muted)] transition-colors group-hover:text-[var(--accent)]">
                      <IconComp size={14} />
                    </span>
                    <ArrowUpRight
                      size={13}
                      className="text-[var(--muted)]/40 transition-all group-hover:translate-x-0.5 group-hover:-translate-y-0.5 group-hover:text-[var(--accent)]"
                    />
                  </div>
                  <div className="flex flex-col gap-0.5">
                    <span className="text-[13px] font-medium text-[var(--fg)]">
                      {starter.title}
                    </span>
                    <span className="text-[11.5px] leading-snug text-[var(--muted)]">
                      {starter.subtitle}
                    </span>
                  </div>
                </button>
              )
            })}
          </div>
        </motion.div>
      )}

      {(() => {
        const showDays = chatSlot.show_day_separators !== false
        let prevDay = ''
        let prevUserPrompt = ''
        let prevUserParentId: string | null | undefined = undefined
        let lastAssistantIdx = -1
        historicalBlocks.forEach((b, i) => {
          if (b.kind === 'assistant') lastAssistantIdx = i
        })

        return historicalBlocks.map((block, blockIdx) => {
          const blockIso =
            block.kind === 'user'
              ? block.event.created_at
              : block.kind === 'assistant'
                ? block.group.startedAt
                : undefined
          const dayLabel = showDays && blockIso ? formatDayBucket(blockIso) : ''
          const renderDay = Boolean(dayLabel && dayLabel !== prevDay)
          if (dayLabel) prevDay = dayLabel

          const dayNode = renderDay ? (
            <DayDivider key={`day-${block.kind}-${blockIso}`} label={dayLabel} />
          ) : null

          if (block.kind === 'compaction_checkpoint') {
            return (
              <CompactionCheckpointPill
                key={block.id}
                compacted_events_count={block.compacted_events_count}
                estimated_tokens_before={block.estimated_tokens_before}
                estimated_tokens_after={block.estimated_tokens_after}
                model={block.model}
                summary={block.summary}
              />
            )
          }
          if (block.kind === 'user') {
            const ev = block.event
            const text = String(ev.content_json?.text || '')
            prevUserPrompt = text
            prevUserParentId = ev.parent_id ?? null
            const sibs = siblings[ev.id] || []
            const idx = sibs.indexOf(ev.id)
            return (
              <div key={ev.id} className="zenko-turn-item flex flex-col gap-5 sm:-mx-12 sm:px-12">
                {dayNode}
                <div
                  data-slot="message"
                  data-from="user"
                  className="group relative flex flex-col items-end"
                >
                  <div className="mb-1.5 mr-10 flex items-center gap-2 text-[11.5px] text-[var(--muted)] sm:mr-0">
                    <span className="font-medium text-[var(--fg)]/80">You</span>
                    {ui.widgets.show_timestamps && (
                      <span className="text-[var(--muted)]/75">{formatTimeShort(ev.created_at)}</span>
                    )}
                  </div>
                  <div className="relative flex max-w-[82%] items-start gap-2.5 sm:block">
                    {chatSlot.user_bubble === 'easeui_bubble' ? (
                      <EaseMessageBubble align="end" tone="neutral" className="whitespace-pre-wrap">
                        {text}
                      </EaseMessageBubble>
                    ) : (
                      <div
                        data-slot="message-bubble-content"
                        className={cn('text-[14.5px] leading-relaxed whitespace-pre-wrap', userBubbleClass)}
                      >
                        {text}
                      </div>
                    )}
                    {chatSlot.show_avatars && chatSlot.user_avatar !== 'hidden' && (
                      <div className="mt-0.5 shrink-0 sm:absolute sm:left-full sm:top-0.5 sm:ml-3 sm:mt-0">
                        <UserAvatar
                          variant={chatSlot.user_avatar || 'icon_user'}
                          frame={chatSlot.user_avatar_frame || 'circle'}
                          color={chatSlot.user_avatar_color}
                          icons={ui.icons}
                          size={28}
                        />
                      </div>
                    )}
                  </div>
                  <div className="mt-1 mr-10 flex items-center gap-2 text-[11px] text-[var(--muted)] opacity-0 transition-opacity group-hover:opacity-100 sm:mr-0">
                    <button
                      type="button"
                      onClick={() => onEditBranch(ev.parent_id ?? null, text)}
                      className="inline-flex cursor-pointer items-center gap-1 hover:text-[var(--fg)]"
                    >
                      <Edit3 size={11.5} /> Edit
                    </button>
                    {sibs.length > 1 && (
                      <span className="inline-flex items-center gap-1 font-mono">
                        <button
                          type="button"
                          disabled={idx <= 0}
                          onClick={() => idx > 0 && onSwitchBranch(sibs[idx - 1])}
                          className="cursor-pointer disabled:opacity-30"
                        >
                          <ChevronLeft size={12} />
                        </button>
                        {idx + 1}/{sibs.length}
                        <button
                          type="button"
                          disabled={idx >= sibs.length - 1}
                          onClick={() => idx < sibs.length - 1 && onSwitchBranch(sibs[idx + 1])}
                          className="cursor-pointer disabled:opacity-30"
                        >
                          <ChevronRight size={12} />
                        </button>
                      </span>
                    )}
                  </div>
                </div>
              </div>
            )
          }

          const promptForRegen = prevUserPrompt
          const parentForRegen = prevUserParentId
          const matchedCheckpoint =
            checkpoints.find(
              (cp) => !cp.is_safety_backup && (cp.event_id ?? null) === (parentForRegen ?? null),
            ) ?? null
          return (
            <div key={block.group.key} className="zenko-turn-item flex flex-col gap-5 sm:-mx-12 sm:px-12">
              {dayNode}
              <AssistantTurnView
                sessionId={sessionId}
                group={block.group}
                fallbackTasks={
                  blockIdx === lastAssistantIdx && !hasLiveTurn && !block.group.plan
                    ? sessionTasks
                    : undefined
                }
                ui={ui}
                promptForRegen={promptForRegen}
                parentForRegen={parentForRegen}
                checkpoints={checkpoints}
                turnCheckpointId={matchedCheckpoint?.id ?? null}
                showCheckpointCount={blockIdx === lastAssistantIdx}
                onRegenerate={onRegenerate}
                onRetryTool={onRetryTool}
                onRewindCheckpoint={onRewindCheckpoint}
              />
            </div>
          )
        })
      })()}

      {/* Single Unified Live Streaming Turn — zero duplicate Zenko headers! */}
      {hasLiveTurn && (
        <LiveAssistantTurn
          persistedGroup={activePersistedGroup}
          sessionTasks={sessionTasks}
          liveOrchestrator={liveOrchestrator}
          liveThinking={liveThinking}
          liveText={liveText}
          liveTools={liveTools}
          isRunning={isRunning}
          turnStartedAt={turnStartedAt}
          activeModel={activeModel}
          ui={ui}
        />
      )}

      {/* Agent Error State (customizable via chat_page.error_state_style) */}
      {liveTurnError && onRetryTurn && (
        <ErrorStateDisplay
          style={chatSlot.error_state_style}
          title={liveTurnError.title}
          detail={liveTurnError.detail}
          retrying={liveTurnError.retrying}
          onRetry={onRetryTurn}
        />
      )}

      {/* User Stopped Run State (customizable via chat_page.stopped_run_style) */}
      {stoppedRun && !isRunning && (
        <StoppedRunDisplay
          style={chatSlot.stopped_run_style}
          words={stoppedRun.words}
          reason={stoppedRun.reason}
          onContinue={onContinueStopped}
          onDiscard={onDiscardStopped}
        />
      )}

      {/* Interactive Ask-User Prompts (@assistant-ui Elements) */}
      {onResolveClarification &&
        pendingClarifications.map((item) => (
          <AskUserCard key={item.id} item={item} onResolve={onResolveClarification} />
        ))}

      {/* HITL Tool Approval Cards — PermissionGrant (Default) | BeUI ToolApproval | Compact Card */}
      {pendingApprovals.map((apr) => (
        <ApprovalElement
          key={apr.id}
          approval={apr}
          style={chatSlot.approval_style}
          onResolve={onResolveApproval}
        />
      ))}

      <div ref={bottomRef} />
    </div>
  )

  if (scrollerStyle === 'beui_rail' || scrollerStyle === 'beui_scroller') {
    return (
      <MessageScroller
        key={sessionId || 'new'}
        navigation={scrollerStyle === 'beui_rail' ? 'rail' : undefined}
        followOutput
        smooth
        busy={isRunning}
        className="min-h-0 flex-1"
        viewportClassName="px-5 pb-6 pt-8 sm:px-14 md:px-16 scrollbar-hide"
      >
        {streamContent}
      </MessageScroller>
    )
  }

  return (
    <CompactScrollArea
      key={sessionId || 'new'}
      smooth={scrollerStyle === 'smooth'}
      className="min-h-0 flex-1"
      viewportClassName="px-5 pb-6 pt-8 sm:px-14 md:px-16"
    >
      {streamContent}
    </CompactScrollArea>
  )
}
