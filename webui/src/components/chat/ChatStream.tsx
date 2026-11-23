import { useEffect, useMemo, useRef, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import {
  ArrowUpRight,
  BrainIcon,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Clock,
  Edit3,
  FolderSearch,
  ListChecks,
  Search,
  Sliders,
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
  BrainstormSpec,
  ClarificationDecisionNote,
  ClarificationRecord,
  ComponentSlots,
  ExecutionPlan,
  OrchestratorStatus,
  PlanStepStatus,
  SessionEvent,
  UiCustomization,
} from '@/lib/types'
import { cn } from '@/lib/utils'
import type { AgentActivityItem } from '@/components/agents/agent-activity'
import { MessageScroller } from '@/components/agents/message-scroller'
import { StreamingResponse } from '@/components/agents/streaming-response'
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
import { BouncyAccordion } from '@/components/motion/bouncy-accordion'
import type { LoaderVariant } from '@/components/motion/loader'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { DayDivider } from '@/components/assistant-ui/elements/day-separator'
import { ErrorState } from '@/components/assistant-ui/elements/error-state'
import { StoppedRun } from '@/components/assistant-ui/elements/stopped-run'
import { StreamingText } from '@/components/assistant-ui/elements/streaming-text'
import {
  ActivityDisplay,
  ApprovalElement,
  AssistantMessageFooter,
  LoadingStatusBar,
  MarkdownCodeBlock,
  MarkdownDataTable,
  SkillAndMemoryBadges,
  type NormalizedToolStep,
} from '@/components/chat/ChatElements'

const CHAT_WIDTH_CLASS: Record<ComponentSlots['chat_page']['content_width'], string> = {
  narrow: 'max-w-[680px]',
  balanced: 'max-w-[780px]',
  wide: 'max-w-[960px]',
  full: 'max-w-none',
}

const USER_BUBBLE_CLASS: Record<ComponentSlots['chat_page']['user_bubble'], string> = {
  soft_card: 'rounded-[15px] border border-[var(--border)] bg-[var(--card)] px-4 py-2.5 text-[var(--fg)]',
  accent_tint:
    'rounded-[15px] border border-[var(--accent)]/35 bg-[var(--accent)]/12 px-4 py-2.5 text-[var(--fg)]',
  minimal_border:
    'rounded-[14px] border border-[var(--border)] bg-transparent px-4 py-2.5 text-[var(--fg)]',
  pill: 'rounded-3xl border border-[var(--border)] bg-[var(--card)] px-5 py-2.5 text-[var(--fg)]',
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

function compactArgsPreview(args: unknown): string {
  if (args == null) return ''
  if (typeof args === 'string') return args.slice(0, 120)
  try {
    const s = JSON.stringify(args)
    return s.length > 120 ? `${s.slice(0, 120)}…` : s
  } catch {
    return ''
  }
}

type RenderBlock =
  | { kind: 'user'; event: SessionEvent }
  | { kind: 'brainstorm'; id: string; spec: BrainstormSpec }
  | { kind: 'plan'; id: string; plan: ExecutionPlan }
  | { kind: 'clarification_decision'; id: string; decision: ClarificationDecisionNote }
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

const SYSTEM_NOTE_PARSERS: Record<string, (ev: SessionEvent) => RenderBlock | null> = {
  brainstorm_spec: (ev) =>
    ev.content_json?.spec
      ? { kind: 'brainstorm', id: ev.id, spec: ev.content_json.spec as BrainstormSpec }
      : null,
  execution_plan: (ev) =>
    ev.content_json?.plan
      ? { kind: 'plan', id: ev.id, plan: ev.content_json.plan as ExecutionPlan }
      : null,
  clarification_decision: (ev) =>
    ev.content_json?.decision
      ? {
          kind: 'clarification_decision',
          id: ev.id,
          decision: ev.content_json.decision as ClarificationDecisionNote,
        }
      : null,
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
      const parser = SYSTEM_NOTE_PARSERS[String(ev.content_json?.note_kind || '')]
      const parsed = parser ? parser(ev) : null
      if (parsed) {
        flush()
        blocks.push(parsed)
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
      if (!current) {
        current = {
          key: ev.id,
          model: 'zenko',
          startedAt: ev.created_at,
          timestamp: ev.created_at,
          durationMs: 0,
          thinkingMs: 0,
          thinking: '',
          text: '',
          tools: [],
          tokens: 0,
          cache_read: 0,
          cost_usd: 0,
        }
      }
      const callId = typeof ev.content_json?.tool_call_id === 'string' ? ev.content_json.tool_call_id : undefined
      current.tools.push({
        call_id: callId,
        name: ev.content_json?.tool_name || 'tool',
        args_preview: callId ? toolArgsById.get(callId) : undefined,
        is_error: Boolean(ev.content_json?.is_error),
        duration_ms: Number(ev.content_json?.duration_ms || 0),
        diff: ev.content_json?.diff || null,
        content: String(ev.content_json?.content || ''),
      })
      current.timestamp = ev.created_at
    }
  }
  flush()
  return blocks
}

function MarkdownProse({
  text,
  codeBlockStyle = 'aui_code_runner',
}: {
  text: string
  codeBlockStyle?: ComponentSlots['chat_page']['code_block_style']
}) {
  return (
    <div className="zenko-prose" data-slot="message-content">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          pre({ children }) {
            return <>{children}</>
          },
          code({ className, children }) {
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
          table({ children }) {
            return <MarkdownDataTable>{children}</MarkdownDataTable>
          },
          a({ href, children, ...props }) {
            const safeHref = typeof href === 'string' ? href : undefined
            const isExternal = Boolean(
              safeHref && (safeHref.startsWith('http://') || safeHref.startsWith('https://')),
            )
            return (
              <a
                {...props}
                href={safeHref}
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
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  )
}

function CompactionCheckpointPill({
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
          <div className="zenko-prose text-[12px]">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{summary}</ReactMarkdown>
          </div>
        </div>
      )}
    </div>
  )
}

function AssistantTurnView({
  group,
  ui,
  onRegenerate,
  onRetryTool,
}: {
  group: AssistantGroup
  ui: UiCustomization
  onRegenerate?: () => void
  onRetryTool?: (toolName: string, argsPreview?: string) => void
}) {
  const [stepsOpen, setStepsOpen] = useState(false)
  const hasThinking = Boolean(ui.widgets.show_thinking && group.thinking)
  const stepCount = group.tools.length + (hasThinking ? 1 : 0)
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
    return `Completed ${stepWord}`
  }, [stepCount])

  return (
    <motion.div
      data-slot="message"
      data-from="assistant"
      initial={{ opacity: 0, y: 6 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.2, ease: EASE_OUT }}
      className="group flex items-start gap-3.5"
    >
      {chatSlot.show_avatars && ui.motion.orb_style !== 'hidden' && (
        <div className="mt-0.5 shrink-0">
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
          'min-w-0 flex-1',
          ui.layout.chat_layout === 'bubble_split' &&
            'rounded-2xl border border-[var(--border)] bg-[var(--surface)]/80 p-4',
        )}
      >
        {/* Meta row: Zenko + model pill + quiet time */}
        <div className="mb-2 flex flex-wrap items-center gap-2 text-[12.5px]">
          <span className="font-medium text-[var(--fg)]/85">Zenko</span>
          <span className="rounded-md border border-[var(--border)] bg-[var(--card)] px-2 py-0.5 font-mono text-[11px] text-[var(--muted)]">
            {shortModel}
          </span>
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
            {chatSlot.thinking_style === 'inline_quote' && hasThinking && group.tools.length === 0 ? (
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
                    {/* 1. Thought duration + full reasoning */}
                    {hasThinking && (
                      <div className="flex flex-col gap-1.5">
                        <div className="flex items-center gap-1.5 font-mono text-[10.5px] uppercase tracking-wider text-[var(--muted)]">
                          <BrainIcon size={12} style={{ color: 'var(--accent)' }} />
                          <span>{thoughtLabel ? `Thought for ${thoughtLabel}` : 'Reasoning'}</span>
                        </div>
                        <CompactScrollArea
                          autoHeight
                          className="zenko-aui-paper rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)]"
                          viewportClassName="max-h-56 p-3 pr-4"
                        >
                          <div className="font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-[var(--muted)]">
                            {group.thinking}
                          </div>
                        </CompactScrollArea>
                      </div>
                    )}

                    {/* 2. Unified Agent Activity & Tool Execution (zero duplicate tool lists) */}
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
          (useBeuiInlineActions ? (
            <StreamingResponse
              status="complete"
              copyText={group.text}
              onRetry={onRegenerate}
              sources={citations}
              announce={false}
              contentClassName="text-[0.94rem]"
              footerMeta={tokenMetaNode}
            >
              <MarkdownProse text={group.text} codeBlockStyle={codeBlockStyle} />
            </StreamingResponse>
          ) : (
            <>
              <MarkdownProse text={group.text} codeBlockStyle={codeBlockStyle} />
              <AssistantMessageFooter
                style={actionsStyle}
                text={group.text}
                citations={citations}
                tokenMeta={tokenMetaNode}
                onRegenerate={onRegenerate}
                onToggleSteps={stepCount > 0 ? () => setStepsOpen((v) => !v) : undefined}
              />
            </>
          ))}
      </div>
    </motion.div>
  )
}

function BrainstormCard({
  spec,
  bouncy,
}: {
  spec: BrainstormSpec
  bouncy: boolean
}) {
  const [open, setOpen] = useState(true)
  const body = (
    <div className="flex flex-col gap-3">
      <p className="text-[13px] leading-relaxed text-[var(--fg)]/90">{spec.problem}</p>
      <div className="grid gap-2 sm:grid-cols-2">
        {spec.options.map((opt) => {
          const isSelected = opt.id === spec.selected_option_id
          return (
            <div
              key={opt.id}
              className="rounded-[var(--radius-sm)] border bg-[var(--bg)] p-2.5"
              style={{
                borderColor: isSelected
                  ? 'color-mix(in srgb, var(--accent) 52%, var(--border))'
                  : 'var(--border)',
              }}
            >
              <div className="flex items-center justify-between gap-1.5">
                <span className="font-medium text-[var(--fg)]">{opt.title}</span>
                {isSelected && (
                  <span className="font-mono text-[10px] uppercase" style={{ color: 'var(--accent)' }}>
                    selected
                  </span>
                )}
              </div>
              <p className="mt-1 text-[11.5px] text-[var(--muted)]">{opt.summary}</p>
              <div className="mt-1.5 flex flex-wrap gap-2 font-mono text-[10px] text-[var(--muted)]">
                <span>rev:{opt.reversibility}</span>
                <span>blast:{opt.blast_radius}</span>
                <span>safe:{opt.safety_score}/10</span>
                {opt.touched_files && opt.touched_files.length > 0 && (
                  <span>files:{opt.touched_files.length}</span>
                )}
                {opt.rollback_steps && opt.rollback_steps.length > 0 && <span>rollback:ready</span>}
              </div>
            </div>
          )
        })}
      </div>
      {spec.selection_rationale && (
        <div className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 text-[11.5px] text-[var(--muted)]">
          <strong className="text-[var(--fg)]/85">Rationale:</strong> {spec.selection_rationale}
        </div>
      )}
      {spec.pre_mortem && spec.pre_mortem.length > 0 && (
        <div className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 text-[11.5px] text-[var(--muted)]">
          <strong className="text-[var(--fg)]/85">Pre-Mortem:</strong> {spec.pre_mortem.join(' · ')}
        </div>
      )}
      {spec.verification_plan.length > 0 && (
        <div className="font-mono text-[11px] text-[var(--muted)]">
          Verify: {spec.verification_plan.join(' · ')}
        </div>
      )}
      {spec.rubric_warnings && spec.rubric_warnings.length > 0 && (
        <div className="font-mono text-[10.5px] text-[var(--muted)]">
          Rubric: {spec.rubric_warnings.join(' · ')}
        </div>
      )}
    </div>
  )

  if (bouncy) {
    return (
      <BouncyAccordion
        defaultValue={spec.id}
        collapsible
        items={[
          {
            id: spec.id,
            icon: <BrainIcon size={14} style={{ color: 'var(--accent)' }} />,
            title: (
              <span className="inline-flex items-center gap-2 text-[13px]">
                <span>Structured Brainstorm</span>
                <span className="rounded border border-[var(--border)] bg-[var(--bg)] px-1.5 py-0.5 font-mono text-[10.5px] text-[var(--muted)]">
                  {spec.options.length} options · {spec.selected_option_id}
                </span>
              </span>
            ),
            description: body,
          },
        ]}
        classNames={{
          item: 'border border-[var(--border)] bg-[var(--surface)]/92',
          trigger: 'min-h-[46px] px-4',
          description: 'text-[12.5px]',
        }}
      />
    )
  }

  return (
    <div className="rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)]/90 p-4 text-[12.5px]">
      <div className="flex items-center justify-between gap-2">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="inline-flex cursor-pointer items-center gap-2 text-left font-medium text-[var(--fg)]"
        >
          <BrainIcon size={13.5} style={{ color: 'var(--accent)' }} />
          <span>Structured Brainstorm</span>
          <span className="rounded border border-[var(--border)] bg-[var(--bg)] px-1.5 py-0.5 font-mono text-[10.5px] text-[var(--muted)]">
            {spec.options.length} options · {spec.selected_option_id}
          </span>
        </button>
        <ChevronDown
          size={14}
          className={cn('text-[var(--muted)] transition-transform', open && 'rotate-180')}
        />
      </div>
      {open && <div className="mt-3">{body}</div>}
    </div>
  )
}

const PLAN_STEP_GLYPH: Record<PlanStepStatus, string> = {
  completed: '✓',
  in_progress: '▸',
  pending: '○',
  blocked: '!',
}

function ExecutionPlanCard({ plan, bouncy }: { plan: ExecutionPlan; bouncy: boolean }) {
  const [open, setOpen] = useState(true)
  const done = plan.steps.filter((s) => s.status === 'completed').length
  const total = plan.steps.length
  const pct = total > 0 ? Math.round((done / total) * 100) : 0

  const body = (
    <div className="flex flex-col gap-2">
      <div className="h-1 w-full overflow-hidden rounded-full bg-[var(--bg)]">
        <div
          className="h-full transition-all"
          style={{ width: `${pct}%`, backgroundColor: 'var(--accent)' }}
        />
      </div>
      <p className="text-[12.5px] text-[var(--fg)]/90">{plan.goal}</p>
      <div className="flex flex-col gap-1.5">
        {plan.steps.map((step) => (
          <div
            key={step.id}
            className="flex items-start justify-between gap-2 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-2.5 py-1.5"
          >
            <div className="flex items-start gap-2">
              <span
                className="mt-0.5 font-mono text-[11px]"
                style={{
                  color:
                    step.status === 'in_progress' || step.status === 'completed'
                      ? 'var(--accent)'
                      : 'var(--muted)',
                }}
              >
                {PLAN_STEP_GLYPH[step.status] || '○'}
              </span>
              <div>
                <div
                  className={cn(
                    'text-[12px]',
                    step.status === 'completed'
                      ? 'text-[var(--muted)] line-through'
                      : 'text-[var(--fg)]',
                  )}
                >
                  {step.title}
                </div>
                {step.depends_on.length > 0 && (
                  <div className="font-mono text-[10px] text-[var(--muted)]">
                    after: {step.depends_on.join(', ')}
                  </div>
                )}
              </div>
            </div>
            <span className="shrink-0 font-mono text-[10px] text-[var(--muted)]">
              {step.status}
            </span>
          </div>
        ))}
      </div>
      {plan.rubric_warnings && plan.rubric_warnings.length > 0 && (
        <div className="font-mono text-[10.5px] text-[var(--muted)]">
          Rubric: {plan.rubric_warnings.join(' · ')}
        </div>
      )}
    </div>
  )

  if (bouncy) {
    return (
      <BouncyAccordion
        defaultValue={plan.id}
        collapsible
        items={[
          {
            id: plan.id,
            icon: <ListChecks size={14} style={{ color: 'var(--accent)' }} />,
            title: (
              <span className="inline-flex items-center gap-2 text-[13px]">
                <span>Execution Plan</span>
                <span className="rounded border border-[var(--border)] bg-[var(--bg)] px-1.5 py-0.5 font-mono text-[10.5px] text-[var(--muted)]">
                  {done}/{total} done
                </span>
              </span>
            ),
            description: body,
          },
        ]}
        classNames={{
          item: 'border border-[var(--border)] bg-[var(--surface)]/92',
          trigger: 'min-h-[46px] px-4',
          description: 'text-[12.5px]',
        }}
      />
    )
  }

  return (
    <div className="rounded-[var(--radius)] border border-[var(--border)] bg-[var(--surface)]/90 p-4 text-[12.5px]">
      <div className="flex items-center justify-between gap-2">
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="inline-flex cursor-pointer items-center gap-2 text-left font-medium text-[var(--fg)]"
        >
          <ListChecks size={13.5} style={{ color: 'var(--accent)' }} />
          <span>Execution Plan</span>
          <span className="rounded border border-[var(--border)] bg-[var(--bg)] px-1.5 py-0.5 font-mono text-[10.5px] text-[var(--muted)]">
            {done}/{total} done
          </span>
        </button>
        <ChevronDown
          size={14}
          className={cn('text-[var(--muted)] transition-transform', open && 'rotate-180')}
        />
      </div>
      {open && <div className="mt-2.5">{body}</div>}
    </div>
  )
}

function ClarificationDecisionPill({ decision }: { decision: ClarificationDecisionNote }) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-2 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)]/75 px-3 py-2 text-[12px]">
      <div className="flex items-center gap-2 text-[var(--fg)]/90">
        <Check size={13} style={{ color: 'var(--accent)' }} />
        <span className="font-medium">{decision.question}</span>
        <span className="rounded border border-[var(--border)] bg-[var(--bg)] px-1.5 py-0.5 font-mono text-[10.5px] text-[var(--fg)]">
          {decision.selected_option_id}
        </span>
      </div>
      <span className="font-mono text-[10.5px] text-[var(--muted)]">
        {decision.resolved_by}
        {decision.custom_answer ? ` · ${decision.custom_answer}` : ''}
      </span>
    </div>
  )
}

function ClarificationCard({
  item,
  onResolve,
}: {
  item: ClarificationRecord
  onResolve: (id: string, selectedOptionId: string, customAnswer?: string) => void
}) {
  const [custom, setCustom] = useState('')
  return (
    <div
      className="rounded-[var(--radius)] border p-4"
      style={{
        borderColor: 'color-mix(in srgb, var(--accent) 48%, var(--border))',
        backgroundColor: 'color-mix(in srgb, var(--accent) 6%, var(--card))',
      }}
    >
      <div className="mb-1.5 flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 text-[13.5px] font-semibold text-[var(--fg)]">
          <Sliders size={14} style={{ color: 'var(--accent)' }} />
          <span>Design Decision Needed</span>
        </div>
        <span className="rounded-full border border-[var(--border)] px-2 py-0.5 font-mono text-[10.5px] text-[var(--muted)]">
          rec: {item.recommended_option_id}
        </span>
      </div>
      <p className="text-[13.5px] font-medium text-[var(--fg)]">{item.question}</p>
      {item.context && <p className="mt-1 text-[12px] text-[var(--muted)]">{item.context}</p>}
      <div className="mt-3 grid gap-2 sm:grid-cols-2">
        {item.options.map((opt) => (
          <button
            key={opt.id}
            type="button"
            onClick={() => onResolve(item.id, opt.id, custom.trim() || undefined)}
            className="flex cursor-pointer flex-col rounded-[var(--radius-sm)] border bg-[var(--bg)] p-2.5 text-left transition-colors hover:border-[var(--accent)]"
            style={{
              borderColor: opt.recommended
                ? 'color-mix(in srgb, var(--accent) 45%, var(--border))'
                : 'var(--border)',
            }}
          >
            <div className="flex items-center justify-between gap-2">
              <span className="text-[12.5px] font-medium text-[var(--fg)]">{opt.label}</span>
              {opt.recommended && (
                <span className="font-mono text-[10px] uppercase" style={{ color: 'var(--accent)' }}>
                  recommended
                </span>
              )}
            </div>
            {opt.description && (
              <span className="mt-0.5 text-[11.5px] text-[var(--muted)]">{opt.description}</span>
            )}
          </button>
        ))}
      </div>
      <input
        value={custom}
        onChange={(e) => setCustom(e.target.value)}
        placeholder="Optional extra constraint before picking an option..."
        className="mt-2.5 w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-1.5 text-[12px] text-[var(--fg)] focus:border-[var(--accent)] focus:outline-none"
      />
    </div>
  )
}

function LiveAssistantTurn({
  persistedGroup,
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
  const liveStatusText = useMemo(() => {
    if (runningTool) return `Running ${runningTool.tool_name}...`
    if (liveText) return `Writing response · ${mergedTools.length} ${mergedTools.length === 1 ? 'tool' : 'tools'}`
    if (mergedThinking) return 'Thinking...'
    if (liveOrchestrator?.stage) return `${liveOrchestrator.stage}...`
    return 'Thinking...'
  }, [runningTool, liveText, mergedTools.length, mergedThinking, liveOrchestrator?.stage])

  const hasWorkDetails =
    Boolean(ui.widgets.show_thinking && mergedThinking) ||
    mergedTools.length > 0 ||
    liveActivityItems.length > 0

  return (
    <div data-slot="message" data-from="assistant" className="flex items-start gap-3.5">
      {chatSlot.show_avatars && ui.motion.orb_style !== 'hidden' && (
        <div className="mt-0.5 shrink-0">
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
      <div className="min-w-0 flex-1">
        {/* Single Unified Zenko Header */}
        <div className="mb-2 flex flex-wrap items-center gap-2 text-[12.5px]">
          <span className="font-medium text-[var(--fg)]/85">Zenko</span>
          <span className="rounded-md border border-[var(--border)] bg-[var(--card)] px-2 py-0.5 font-mono text-[11px] text-[var(--muted)]">
            {shortModel}
          </span>
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
                <div className="flex flex-col gap-2.5 border-l border-[var(--border)] pl-3.5">
                  {/* Live streaming reasoning words */}
                  {ui.widgets.show_thinking && mergedThinking && (
                    <CompactScrollArea
                      autoHeight
                      stickToBottom
                      className="zenko-aui-paper rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)]"
                      viewportClassName="max-h-44 p-3 pr-4"
                    >
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
                  )}

                  {/* Unified Live Agent Activity & Tool Execution */}
                  {chatSlot.activity_style !== 'hidden' &&
                    (liveActivityItems.length > 0 || mergedTools.length > 0) && (
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
                      />
                    )}
                </div>
              </AgentDisclosure>
            )}
          </div>
        )}

        {/* Live Streaming Response Words (StreamingText | BeUI StreamingResponse | Editorial) */}
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
          ) : chatSlot.streaming_style === 'beui_stream' ? (
            <StreamingResponse status="streaming" announce={false} showActions={false}>
              <MarkdownProse
                text={mergedText}
                codeBlockStyle={chatSlot.code_block_style || 'aui_code_runner'}
              />
            </StreamingResponse>
          ) : (
            <MarkdownProse
              text={mergedText}
              codeBlockStyle={chatSlot.code_block_style || 'aui_code_runner'}
            />
          ))}
      </div>
    </div>
  )
}

export function ChatStream({
  events,
  siblings,
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
  onOpenSetup,
  onEditBranch,
  onSwitchBranch,
  onResolveApproval,
  onResolveClarification,
}: {
  events: SessionEvent[]
  siblings: Record<string, string[]>
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
  onOpenSetup: () => void
  onEditBranch: (parentId: string | null, text: string) => void
  onSwitchBranch: (eventId: string) => void
  onResolveApproval: (id: string, decision: 'approve' | 'deny' | 'always_allow') => void
  onResolveClarification?: (id: string, selectedOptionId: string, customAnswer?: string) => void
}) {
  const bottomRef = useRef<HTMLDivElement>(null)
  const blocks = useMemo(() => groupEvents(events), [events])
  const chatSlot = ui.components.chat_page
  const scrollerStyle = chatSlot.scroller_style || 'beui_rail'
  const bouncyCards = ui.surface.accordion_style !== 'compact'

  // Unified single-turn merge: if the session is actively running and earlier iterations
  // of THIS turn were already persisted into SQLite, merge that trailing assistant block
  // into LiveAssistantTurn so we NEVER render duplicate Zenko headers for one prompt!
  const hasLiveTurn = isRunning || Boolean(liveText || liveThinking || liveTools.length > 0)
  const lastBlock = blocks[blocks.length - 1]
  const mergeLastIntoLive = hasLiveTurn && lastBlock?.kind === 'assistant'
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
              {ui.surface.alert_style !== 'compact' ? (
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

        return historicalBlocks.map((block) => {
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

          if (block.kind === 'brainstorm') {
            return <BrainstormCard key={block.id} spec={block.spec} bouncy={bouncyCards} />
          }
          if (block.kind === 'plan') {
            return <ExecutionPlanCard key={block.id} plan={block.plan} bouncy={bouncyCards} />
          }
          if (block.kind === 'clarification_decision') {
            return <ClarificationDecisionPill key={block.id} decision={block.decision} />
          }
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
              <div key={ev.id} className="flex flex-col gap-5">
                {dayNode}
                <div
                  data-slot="message"
                  data-from="user"
                  className="group flex flex-col items-end"
                >
                  <div className="mb-1.5 mr-10 flex items-center gap-2 text-[11.5px] text-[var(--muted)]">
                    <span className="font-medium text-[var(--fg)]/80">You</span>
                    {ui.widgets.show_timestamps && (
                      <span className="text-[var(--muted)]/75">{formatTimeShort(ev.created_at)}</span>
                    )}
                  </div>
                  <div className="flex max-w-[82%] items-start gap-2.5">
                    <div
                      data-slot="message-bubble-content"
                      className={cn('text-[14.5px] leading-relaxed whitespace-pre-wrap', userBubbleClass)}
                    >
                      {text}
                    </div>
                    {chatSlot.show_avatars && chatSlot.user_avatar !== 'hidden' && (
                      <div className="mt-0.5 shrink-0">
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
                  <div className="mt-1 mr-10 flex items-center gap-2 text-[11px] text-[var(--muted)] opacity-0 transition-opacity group-hover:opacity-100">
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
          return (
            <div key={block.group.key} className="flex flex-col gap-5">
              {dayNode}
              <AssistantTurnView
                group={block.group}
                ui={ui}
                onRegenerate={
                  onRegenerate && promptForRegen
                    ? () => onRegenerate(promptForRegen, parentForRegen)
                    : undefined
                }
                onRetryTool={onRetryTool}
              />
            </div>
          )
        })
      })()}

      {/* Single Unified Live Streaming Turn — zero duplicate Zenko headers! */}
      {hasLiveTurn && (
        <LiveAssistantTurn
          persistedGroup={activePersistedGroup}
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

      {/* Agent Error State (Assistant-UI ErrorState with one-click Retry) */}
      {liveTurnError && onRetryTurn && (
        <ErrorState
          title={liveTurnError.title}
          detail={liveTurnError.detail}
          retrying={liveTurnError.retrying}
          onRetry={onRetryTurn}
        />
      )}

      {/* User Stopped Run State (Assistant-UI StoppedRun with Continue / Discard) */}
      {stoppedRun && !isRunning && (
        <StoppedRun
          words={stoppedRun.words}
          reason={stoppedRun.reason}
          onContinue={onContinueStopped}
          onDiscard={onDiscardStopped}
        />
      )}

      {/* Interactive Orchestrator Clarification Cards */}
      {onResolveClarification &&
        pendingClarifications.map((item) => (
          <ClarificationCard key={item.id} item={item} onResolve={onResolveClarification} />
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
        navigation={scrollerStyle === 'beui_rail' ? 'rail' : undefined}
        followOutput
        smooth
        busy={isRunning}
        className="min-h-0 flex-1"
        viewportClassName="px-5 pb-6 pt-8 sm:px-8 scrollbar-hide"
      >
        {streamContent}
      </MessageScroller>
    )
  }

  return (
    <CompactScrollArea
      smooth={scrollerStyle === 'smooth'}
      className="min-h-0 flex-1"
      viewportClassName="px-5 pb-6 pt-8 sm:px-8"
    >
      {streamContent}
    </CompactScrollArea>
  )
}
