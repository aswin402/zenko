import { useEffect, useState } from 'react'
import { RefreshCw, Search, Trash2 } from 'lucide-react'
import { api, formatCompactTokens } from '@/lib/api'
import type {
  BackgroundJob,
  DaemonTelemetry,
  JobReadChunk,
  LogCategoryMeta,
  LogEntry,
  SubagentRecord,
  SubagentRoleDescriptor,
  TaskKindMeta,
  WorktreeDiffReport,
} from '@/lib/types'
import { Tabs, TabsList, TabsTrigger } from '@/components/motion/tabs'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { ScrollPage, type SecondaryTabsVariant } from './shared'

// ─── Telemetry & Categorized Persistent Logs View ─────────────────────────────

const LOG_LEVEL_FILTERS = ['all', 'INFO', 'WARN', 'ERROR'] as const

const JOB_STATUS_COLOR: Record<BackgroundJob['status'], string> = {
  running: 'var(--accent)',
  completed: 'var(--muted)',
  stopped: 'var(--muted)',
  failed: 'var(--destructive)',
  interrupted: 'var(--destructive)',
}

// Strict 60/30/10 log channel palette (neutral foreground/muted + single global accent + destructive error only)
const CHANNEL_TAG_COLOR: Record<string, string> = {
  input: 'var(--fg)',
  agent: 'var(--accent)',
  tool: 'var(--fg)',
  approve: 'var(--accent)',
  gateway: 'var(--fg)',
  tui: 'var(--accent)',
  http: 'var(--muted)',
  cron: 'var(--muted)',
  system: 'var(--muted)',
}

const GLYPH_COLOR: Record<string, string> = {
  '<--': 'var(--muted)',
  '···': 'var(--accent)',
  '-->': 'var(--accent)',
  '  !': 'var(--accent)',
  '  ×': 'var(--destructive)',
}

const STATUS_COLOR_BY_CLASS: Record<number, string> = {
  2: 'var(--fg)',
  3: 'var(--muted)',
  4: 'var(--accent)',
  5: 'var(--destructive)',
}

const STATUS_COLOR_BY_TOKEN: Record<string, string> = {
  ok: 'var(--fg)',
  done: 'var(--fg)',
  ready: 'var(--fg)',
  approved: 'var(--fg)',
  always: 'var(--fg)',
  connected: 'var(--fg)',
  enabled: 'var(--muted)',
  cron: 'var(--muted)',
  heartbeat: 'var(--muted)',
  moderate: 'var(--accent)',
  dangerous: 'var(--accent)',
  retry: 'var(--accent)',
  stopping: 'var(--accent)',
  vault: 'var(--accent)',
  err: 'var(--destructive)',
  denied: 'var(--destructive)',
  idle: 'var(--muted)',
  stopped: 'var(--muted)',
}

const LEVEL_FALLBACK_COLOR: Record<string, string> = {
  ERROR: 'var(--destructive)',
  WARN: 'var(--accent)',
  INFO: 'var(--muted)',
}

function resolveStatusColor(status: string, level: string): string {
  const byToken = STATUS_COLOR_BY_TOKEN[status.toLowerCase()]
  const byHttpClass = STATUS_COLOR_BY_CLASS[Math.floor(Number(status) / 100)]
  return byToken || byHttpClass || LEVEL_FALLBACK_COLOR[level] || 'var(--muted)'
}

function formatLogTime(ts: string): string {
  const d = new Date(ts)
  if (Number.isNaN(d.getTime())) return ''
  return d.toTimeString().slice(0, 8)
}

function HonoLogRow({
  entry,
  onSelectSession,
}: {
  entry: LogEntry
  onSelectSession: (sid: string) => void
}) {
  const [expanded, setExpanded] = useState(false)
  const hasDetails = Boolean(entry.payload || entry.turn_id || entry.call_id)
  const cols = entry.message.split('\t')
  const shortSid = entry.session_id ? entry.session_id.slice(0, 8) : null
  const timeStr = formatLogTime(entry.timestamp)

  const rowHeader =
    cols.length === 5 ? (
      (() => {
        const [glyph, tag, subject, status, meta] = cols
        const glyphColor = GLYPH_COLOR[glyph] || 'var(--muted)'
        const tagColor = CHANNEL_TAG_COLOR[entry.target] || 'var(--muted)'
        const statusColor = resolveStatusColor(status, entry.level)
        return (
          <div className="flex items-baseline gap-2 py-[2.5px] font-mono text-[11.5px] leading-snug">
            <span className="w-[54px] shrink-0 select-none text-[10px] text-[var(--muted)]/75">
              {timeStr}
            </span>
            <span className="w-5 shrink-0 select-none text-right" style={{ color: glyphColor }}>
              {glyph.trim()}
            </span>
            <span className="w-[58px] shrink-0 font-semibold tracking-tight" style={{ color: tagColor }}>
              {tag}
            </span>
            {shortSid && (
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation()
                  if (entry.session_id) onSelectSession(entry.session_id)
                }}
                title={`Filter session ${entry.session_id}`}
                className="shrink-0 cursor-pointer rounded border border-[var(--border)] bg-[var(--surface)] px-1 py-px text-[9.5px] text-[var(--muted)] hover:border-[var(--accent)] hover:text-[var(--fg)]"
              >
                {shortSid}
              </button>
            )}
            <span className="min-w-0 flex-1 truncate text-[var(--fg)]/90" title={subject}>
              {subject}
            </span>
            {status && (
              <span className="shrink-0 font-semibold" style={{ color: statusColor }}>
                {status}
              </span>
            )}
            {meta && (
              <span className="shrink-0 text-right text-[var(--muted)]" title={meta}>
                {meta}
              </span>
            )}
          </div>
        )
      })()
    ) : (
      (() => {
        const levelColor = LEVEL_FALLBACK_COLOR[entry.level] || 'var(--accent)'
        return (
          <div className="flex items-baseline gap-2 py-[2.5px] font-mono text-[11.5px] leading-snug text-[var(--muted)]">
            <span className="w-[54px] shrink-0 select-none text-[10px] text-[var(--muted)]/75">
              {timeStr}
            </span>
            <span className="w-5 shrink-0 select-none text-right" style={{ color: levelColor }}>
              {entry.level === 'ERROR' ? '×' : '!'}
            </span>
            <span className="w-[58px] shrink-0 truncate font-semibold uppercase" style={{ color: levelColor }}>
              {entry.level}
            </span>
            <span className="min-w-0 flex-1 truncate text-[var(--fg)]/80" title={entry.message}>
              {entry.target}: {entry.message}
            </span>
          </div>
        )
      })()
    )

  return (
    <div
      onClick={() => hasDetails && setExpanded((v) => !v)}
      className={`rounded px-1.5 transition-colors ${hasDetails ? 'cursor-pointer hover:bg-white/[0.03]' : 'hover:bg-white/[0.015]'}`}
    >
      {rowHeader}
      {expanded && hasDetails && (
        <pre className="mb-1.5 ml-[82px] overflow-x-auto rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)] p-2 font-mono text-[10.5px] leading-relaxed text-[var(--muted)]">
          {JSON.stringify(
            {
              category: entry.category,
              session_id: entry.session_id,
              turn_id: entry.turn_id,
              call_id: entry.call_id,
              payload: entry.payload,
            },
            null,
            2,
          )}
        </pre>
      )}
    </div>
  )
}

const SUBAGENT_STATUS_COLOR: Record<SubagentRecord['status'], string> = {
  running: 'var(--accent)',
  completed: 'var(--fg)',
  killed: 'var(--muted)',
  interrupted: 'var(--muted)',
  failed: 'var(--destructive)',
}

export function TelemetryView({
  telemetry,
  logs,
  jobs = [],
  subagents = [],
  subagentRoles = [],
  taskKinds = [],
  logCategories,
  logCounts,
  logsDir,
  tabsVariant = 'pill',
  smoothScroll = true,
  embedded = false,
  showSubagentsAndJobs = true,
  showLogs = true,
  onKillAgent,
  onMergeAgent,
  onDiscardAgent,
  onPruneWorktrees,
  onMessageAgent,
  onInspectAgent,
  onStopJob,
  onRestartJob,
  onDeleteJob,
  onPruneJobs,
  onQueryLogs,
  onRefresh,
}: {
  telemetry?: DaemonTelemetry
  logs: LogEntry[]
  jobs?: BackgroundJob[]
  subagents?: SubagentRecord[]
  subagentRoles?: SubagentRoleDescriptor[]
  taskKinds?: TaskKindMeta[]
  logCategories: LogCategoryMeta[]
  logCounts: Record<string, number>
  logsDir: string
  tabsVariant?: SecondaryTabsVariant
  smoothScroll?: boolean
  embedded?: boolean
  showSubagentsAndJobs?: boolean
  showLogs?: boolean
  onKillAgent?: (agentId: string) => void
  onMergeAgent?: (agentId: string) => void
  onDiscardAgent?: (agentId: string) => void
  onPruneWorktrees?: () => void
  onMessageAgent?: (agentId: string, message: string, interrupt: boolean) => Promise<void> | void
  onInspectAgent?: (agentId: string) => void
  onStopJob?: (jobId: string) => void
  onRestartJob?: (jobId: string) => void
  onDeleteJob?: (jobId: string) => void
  onPruneJobs?: (kind: string) => void
  onQueryLogs: (params: {
    category: string
    level: string
    sessionId: string
    search: string
  }) => void
  onRefresh: () => void
}) {
  const [categoryFilter, setCategoryFilter] = useState<string>('all')
  const [taskKindFilter, setTaskKindFilter] = useState<string>('all')
  const [levelFilter, setLevelFilter] = useState<string>('all')
  const [sessionFilter, setSessionFilter] = useState<string>('')
  const [searchQuery, setSearchQuery] = useState<string>('')
  const [expandedJobId, setExpandedJobId] = useState<string | null>(null)
  const [jobOutput, setJobOutput] = useState<JobReadChunk | null>(null)
  const [loadingJobOutput, setLoadingJobOutput] = useState(false)
  const [expandedAgentId, setExpandedAgentId] = useState<string | null>(null)
  const [agentDiff, setAgentDiff] = useState<WorktreeDiffReport | null>(null)
  const [loadingAgentDiff, setLoadingAgentDiff] = useState(false)
  const [agentMsgDraft, setAgentMsgDraft] = useState<string>('')
  const [sendingAgentMsg, setSendingAgentMsg] = useState(false)

  const fetchAgentDiff = async (agentId: string) => {
    setLoadingAgentDiff(true)
    try {
      const rep = await api<WorktreeDiffReport>(
        `/api/agents/${encodeURIComponent(agentId)}/diff`,
      )
      setAgentDiff(rep)
    } catch {
      setAgentDiff(null)
    } finally {
      setLoadingAgentDiff(false)
    }
  }

  const fetchJobOutput = async (jobId: string) => {
    setLoadingJobOutput(true)
    try {
      const chunk = await api<JobReadChunk>(`/api/jobs/${encodeURIComponent(jobId)}/output`)
      setJobOutput(chunk)
    } catch {
      setJobOutput(null)
    } finally {
      setLoadingJobOutput(false)
    }
  }

  const rssMb = telemetry ? (telemetry.rss_bytes / (1024 * 1024)).toFixed(1) : '0.0'
  const dbKb = telemetry ? (telemetry.db_size_bytes / 1024).toFixed(1) : '0.0'

  useEffect(() => {
    const timer = window.setTimeout(() => {
      onQueryLogs({
        category: categoryFilter,
        level: levelFilter,
        sessionId: sessionFilter,
        search: searchQuery,
      })
    }, 140)
    return () => window.clearTimeout(timer)
  }, [categoryFilter, levelFilter, sessionFilter, searchQuery])

  const activeCategoryMeta = logCategories.find((c) => c.id === categoryFilter)
  const needle = searchQuery.trim().toLowerCase()
  const visibleLogs = logs.filter((entry) => {
    if (categoryFilter !== 'all' && (entry.category || 'system') !== categoryFilter) return false
    if (levelFilter !== 'all' && entry.level !== levelFilter) return false
    if (sessionFilter && entry.session_id !== sessionFilter) return false
    if (needle) {
      const hay = `${entry.target} ${entry.message} ${entry.payload ? JSON.stringify(entry.payload) : ''}`.toLowerCase()
      if (!hay.includes(needle)) return false
    }
    return true
  })

  const opsSections = (
    <>
      {showSubagentsAndJobs && (
        <div className="z-surface-card flex flex-col gap-2.5">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <strong className="text-[13.5px]">Async Subagents & Git Worktrees ({subagents.length})</strong>
              <span className="font-mono text-[10.5px] text-[var(--muted)]">
                {subagents.filter((a) => a.status === 'running').length} active
              </span>
            </div>
            <div className="flex flex-wrap items-center gap-1.5">
              {subagentRoles.length > 0 && (
                <div className="flex flex-wrap items-center gap-1">
                  {subagentRoles.map((r) => (
                    <span
                      key={r.id}
                      title={r.description}
                      className="rounded border border-[var(--border)] bg-[var(--bg)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--muted)]"
                    >
                      {r.id}
                    </span>
                  ))}
                </div>
              )}
              {onPruneWorktrees && (
                <button
                  type="button"
                  onClick={onPruneWorktrees}
                  className="z-btn z-btn-ghost z-btn-sm font-mono text-[10.5px]"
                >
                  Prune Orphans
                </button>
              )}
            </div>
          </div>
          {subagents.length === 0 ? (
            <div className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2.5 font-mono text-[11px] text-[var(--muted)]">
              No subagents spawned yet. Lead sessions delegate automatically via spawn_agent (explorer, architect, coder, verifier).
            </div>
          ) : (
            <div className="flex flex-col gap-1.5">
              {subagents.slice(0, 20).map((a) => {
                const isOpen = expandedAgentId === a.id
                const indentPx = Math.min(Math.max((a.depth || 1) - 1, 0) * 16, 48)
                return (
                  <div
                    key={a.id}
                    style={{ marginLeft: `${indentPx}px` }}
                    className="flex flex-col gap-2 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 font-mono text-[11.5px]"
                  >
                    <div className="flex items-center justify-between gap-3">
                      <button
                        type="button"
                        onClick={() => {
                          if (isOpen) {
                            setExpandedAgentId(null)
                            setAgentDiff(null)
                          } else {
                            setExpandedAgentId(a.id)
                            setAgentDiff(null)
                            if (a.workspace_mode === 'worktree') fetchAgentDiff(a.id)
                          }
                        }}
                        className="min-w-0 flex-1 cursor-pointer text-left"
                      >
                        <span className="mr-1.5 rounded border border-[var(--border)] bg-[var(--surface)] px-1.5 py-px text-[9.5px] uppercase text-[var(--fg)]">
                          {a.role}
                        </span>
                        <span
                          className="mr-2 font-semibold uppercase"
                          style={{ color: SUBAGENT_STATUS_COLOR[a.status] || 'var(--muted)' }}
                        >
                          {a.status}
                        </span>
                        <span className="truncate text-[var(--fg)]" title={a.goal}>
                          {a.goal}
                        </span>
                        <span className="ml-2 text-[10.5px] text-[var(--muted)]">
                          {formatCompactTokens(a.tree_tokens ?? a.tokens_used)} tok
                          {(a.tree_cost_usd ?? a.cost_usd) > 0
                            ? ` · $${(a.tree_cost_usd ?? a.cost_usd).toFixed(4)}`
                            : ''}
                          {(a.descendant_count ?? 0) > 0 ? ` · +${a.descendant_count} sub` : ''}
                          {(a.resume_count ?? 0) > 0 ? ` · r${a.resume_count}` : ''}
                          {a.worktree_branch ? ` · ${a.worktree_branch}` : ''}
                          {a.changed_files.length > 0 ? ` (${a.changed_files.length} files)` : ''}
                        </span>
                      </button>
                      <div className="flex shrink-0 items-center gap-1.5">
                        {onInspectAgent && (
                          <button
                            type="button"
                            onClick={() => onInspectAgent(a.id)}
                            className="z-btn z-btn-ghost z-btn-sm"
                          >
                            Transcript
                          </button>
                        )}
                        {a.status === 'completed' &&
                          a.workspace_mode === 'worktree' &&
                          a.changed_files.length > 0 &&
                          onMergeAgent && (
                            <button
                              type="button"
                              onClick={() => onMergeAgent(a.id)}
                              className="z-btn z-btn-vermilion-soft z-btn-sm"
                            >
                              Merge
                            </button>
                          )}
                        {a.status !== 'running' &&
                          a.workspace_mode === 'worktree' &&
                          a.worktree_branch &&
                          onDiscardAgent && (
                            <button
                              type="button"
                              onClick={() => onDiscardAgent(a.id)}
                              className="z-btn z-btn-ghost z-btn-sm"
                            >
                              Discard
                            </button>
                          )}
                        {a.status === 'running' && onKillAgent && (
                          <button
                            type="button"
                            onClick={() => onKillAgent(a.id)}
                            className="z-btn z-btn-danger z-btn-sm shrink-0"
                          >
                            Kill
                          </button>
                        )}
                      </div>
                    </div>
                    {isOpen && (
                      <div className="flex flex-col gap-2 border-t border-[var(--border)] pt-2 text-[10.5px]">
                        <div className="flex flex-wrap gap-3 text-[var(--muted)]">
                          <span>id={a.id.slice(0, 12)}</span>
                          <span>parent={a.parent_session_id.slice(0, 12)}</span>
                          <span>depth={a.depth}</span>
                          {a.model && <span>model={a.model}</span>}
                          {(a.descendant_count ?? 0) > 0 && (
                            <span>
                              tree={formatCompactTokens(a.tree_tokens ?? a.tokens_used)} tok / $
                              {(a.tree_cost_usd ?? a.cost_usd).toFixed(4)}
                            </span>
                          )}
                        </div>
                        {onMessageAgent && (
                          <form
                            onSubmit={async (e) => {
                              e.preventDefault()
                              const msg = agentMsgDraft.trim()
                              if (!msg || sendingAgentMsg) return
                              setSendingAgentMsg(true)
                              try {
                                await onMessageAgent(a.id, msg, false)
                                setAgentMsgDraft('')
                              } finally {
                                setSendingAgentMsg(false)
                              }
                            }}
                            className="flex items-center gap-1.5"
                          >
                            <input
                              value={agentMsgDraft}
                              onChange={(e) => setAgentMsgDraft(e.target.value)}
                              placeholder={
                                a.status === 'running'
                                  ? 'Steer running subagent...'
                                  : 'Send follow-up to resume this subagent in its session & worktree...'
                              }
                              className="min-w-0 flex-1 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1 font-mono text-[11px] text-[var(--fg)] focus:border-[var(--accent)] focus:outline-none"
                            />
                            <button
                              type="submit"
                              disabled={sendingAgentMsg || !agentMsgDraft.trim()}
                              className="z-btn z-btn-vermilion-soft z-btn-sm shrink-0"
                            >
                              {a.status === 'running' ? 'Steer' : 'Resume'}
                            </button>
                            {a.status === 'running' && (
                              <button
                                type="button"
                                disabled={sendingAgentMsg || !agentMsgDraft.trim()}
                                onClick={async () => {
                                  const msg = agentMsgDraft.trim()
                                  if (!msg || sendingAgentMsg) return
                                  setSendingAgentMsg(true)
                                  try {
                                    await onMessageAgent(a.id, msg, true)
                                    setAgentMsgDraft('')
                                  } finally {
                                    setSendingAgentMsg(false)
                                  }
                                }}
                                className="z-btn z-btn-danger z-btn-sm shrink-0"
                              >
                                Interrupt
                              </button>
                            )}
                          </form>
                        )}
                        {a.structured_output ? (
                          <CompactScrollArea
                            autoHeight
                            className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)]"
                            viewportClassName="max-h-44 p-2 pr-3"
                          >
                            <pre className="whitespace-pre-wrap text-[var(--fg)]/90">
                              {JSON.stringify(a.structured_output, null, 2)}
                            </pre>
                          </CompactScrollArea>
                        ) : (
                          a.summary && (
                            <CompactScrollArea
                              autoHeight
                              className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)]"
                              viewportClassName="max-h-44 p-2 pr-3"
                            >
                              <pre className="whitespace-pre-wrap text-[var(--fg)]/90">
                                {a.summary}
                              </pre>
                            </CompactScrollArea>
                          )
                        )}
                        {a.workspace_mode === 'worktree' && (
                          <div className="flex flex-col gap-1">
                            <span className="text-[var(--muted)]">
                              Worktree diff ({loadingAgentDiff ? 'loading...' : agentDiff?.branch || a.worktree_branch}
                              {agentDiff?.dirty_baseline ? ' · dirty baseline' : ''})
                            </span>
                            {agentDiff?.patch_preview && (
                              <CompactScrollArea
                                autoHeight
                                className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)]"
                                viewportClassName="max-h-48 p-2 pr-3"
                              >
                                <pre className="whitespace-pre-wrap text-[var(--fg)]/85">
                                  {agentDiff.patch_preview}
                                </pre>
                              </CompactScrollArea>
                            )}
                          </div>
                        )}
                      </div>
                    )}
                  </div>
                )
              })}
            </div>
          )}
        </div>
      )}

      {showSubagentsAndJobs && jobs.length > 0 && (
        <div className="z-surface-card flex flex-col gap-2.5">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="flex items-center gap-2">
                <strong className="text-[13.5px]">ZenkoTasks Control Plane ({jobs.length})</strong>
                <span className="font-mono text-[10.5px] text-[var(--muted)]">
                  {jobs.filter((j) => j.status === 'running').length} active
                </span>
              </div>
              {onPruneJobs && (
                <button
                  type="button"
                  onClick={() => onPruneJobs(taskKindFilter)}
                  className="z-btn z-btn-ghost z-btn-sm font-mono text-[10.5px]"
                >
                  Prune Done
                </button>
              )}
            </div>
            {taskKinds.length > 0 && (
              <Tabs variant={tabsVariant} value={taskKindFilter} onValueChange={setTaskKindFilter}>
                <TabsList>
                  <TabsTrigger value="all" className="font-mono text-[11px]">
                    All
                  </TabsTrigger>
                  {taskKinds.map((tk) => {
                    const cnt = jobs.filter((j) => (j.kind || 'shell') === tk.id).length
                    return (
                      <TabsTrigger key={tk.id} value={tk.id} title={tk.description} className="font-mono text-[11px]">
                        <span>{tk.label}</span>
                        {cnt > 0 && <span className="ml-1 opacity-65">{cnt}</span>}
                      </TabsTrigger>
                    )
                  })}
                </TabsList>
              </Tabs>
            )}
            <div className="flex flex-col gap-1.5">
              {jobs
                .filter((j) => taskKindFilter === 'all' || (j.kind || 'shell') === taskKindFilter)
                .slice(0, 16)
                .map((j) => {
                  const isOpen = expandedJobId === j.id
                  return (
                    <div
                      key={j.id}
                      className="flex flex-col gap-2 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 font-mono text-[11.5px]"
                    >
                      <div className="flex items-center justify-between gap-3">
                        <button
                          type="button"
                          onClick={() => {
                            if (isOpen) {
                              setExpandedJobId(null)
                              setJobOutput(null)
                            } else {
                              setExpandedJobId(j.id)
                              fetchJobOutput(j.id)
                            }
                          }}
                          className="min-w-0 flex-1 cursor-pointer text-left"
                        >
                          <span className="mr-1.5 rounded border border-[var(--border)] px-1 py-px text-[9.5px] uppercase text-[var(--muted)]">
                            {j.kind || 'shell'}
                          </span>
                          <span
                            className="mr-2 font-semibold uppercase"
                            style={{ color: JOB_STATUS_COLOR[j.status] || 'var(--muted)' }}
                          >
                            {j.status}
                          </span>
                          <span className="truncate text-[var(--fg)]" title={j.command}>
                            {j.label ? `${j.label} · ${j.command}` : j.command}
                          </span>
                          <span className="ml-2 text-[10.5px] text-[var(--muted)]">
                            {j.bytes_written}B{j.exit_code != null ? ` · exit=${j.exit_code}` : ''}
                          </span>
                        </button>
                        <div className="flex shrink-0 items-center gap-1.5">
                          {isOpen && (
                            <button
                              type="button"
                              onClick={() => fetchJobOutput(j.id)}
                              className="z-btn z-btn-ghost z-btn-sm"
                            >
                              <RefreshCw size={11} className={loadingJobOutput ? 'animate-spin' : ''} />
                            </button>
                          )}
                          {onRestartJob && (
                            <button
                              type="button"
                              onClick={() => onRestartJob(j.id)}
                              className="z-btn z-btn-ghost z-btn-sm"
                            >
                              Restart
                            </button>
                          )}
                          {j.status === 'running' && onStopJob && (
                            <button
                              type="button"
                              onClick={() => onStopJob(j.id)}
                              className="z-btn z-btn-danger z-btn-sm shrink-0"
                            >
                              Stop
                            </button>
                          )}
                          {j.status !== 'running' && onDeleteJob && (
                            <button
                              type="button"
                              onClick={() => onDeleteJob(j.id)}
                              className="z-btn z-btn-ghost z-btn-sm"
                              title="Delete task and log file"
                            >
                              <Trash2 size={11.5} />
                            </button>
                          )}
                        </div>
                      </div>
                      {isOpen && (
                        <CompactScrollArea
                          autoHeight
                          className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)]"
                          viewportClassName="max-h-48 p-2 pr-3"
                        >
                          <pre className="whitespace-pre-wrap text-[10.5px] text-[var(--fg)]/88">
                            {jobOutput?.content || (loadingJobOutput ? 'Loading output...' : '(no output yet)')}
                          </pre>
                        </CompactScrollArea>
                      )}
                    </div>
                  )
                })}
          </div>
        </div>
      )}

      {showLogs && (
        <div className="z-surface-card flex flex-col gap-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <strong className="text-[13.5px]">Categorized Flight Recorder</strong>
              {activeCategoryMeta && (
                <span className="font-mono text-[10.5px] text-[var(--muted)]">
                  {activeCategoryMeta.file}
                </span>
              )}
            </div>
            <span className="font-mono text-[10.5px] text-[var(--muted)]">
              Click any entry with metadata to inspect JSON
            </span>
          </div>

          <Tabs variant={tabsVariant} value={categoryFilter} onValueChange={setCategoryFilter}>
            <TabsList>
              <TabsTrigger value="all" className="font-mono text-[11px]">
                All
              </TabsTrigger>
              {logCategories.map((cat) => {
                const count = logCounts[cat.id] || 0
                return (
                  <TabsTrigger
                    key={cat.id}
                    value={cat.id}
                    title={`${cat.description} (${cat.file})`}
                    className="font-mono text-[11px]"
                  >
                    <span>{cat.label}</span>
                    {count > 0 && <span className="ml-1 opacity-65">{count}</span>}
                  </TabsTrigger>
                )
              })}
            </TabsList>
          </Tabs>

          <div className="flex flex-wrap items-center gap-2">
            <div className="relative min-w-[200px] flex-1">
              <Search size={12.5} className="absolute left-2.5 top-2.5 text-[var(--muted)]" />
              <input
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                placeholder="Search message, tool, path, or JSON payload..."
                className="w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] py-1.5 pl-8 pr-3 font-mono text-[11.5px] text-[var(--fg)] focus:border-[var(--accent)] focus:outline-none"
              />
            </div>
            <Tabs variant={tabsVariant} value={levelFilter} onValueChange={setLevelFilter}>
              <TabsList>
                {LOG_LEVEL_FILTERS.map((lvl) => (
                  <TabsTrigger key={lvl} value={lvl} className="font-mono text-[10.5px]">
                    {lvl === 'all' ? 'All Levels' : lvl}
                  </TabsTrigger>
                ))}
              </TabsList>
            </Tabs>
            {sessionFilter && (
              <button
                type="button"
                onClick={() => setSessionFilter('')}
                className="z-btn z-btn-vermilion-soft z-btn-sm font-mono text-[10.5px]"
              >
                session:{sessionFilter.slice(0, 8)} ×
              </button>
            )}
          </div>

          <CompactScrollArea
            className="h-[440px] rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)]"
            viewportClassName="p-2.5 pr-3.5"
          >
            {visibleLogs.length === 0 ? (
              <div className="p-2 font-mono text-[11.5px] text-[var(--muted)]">
                No log entries match the active filters.
              </div>
            ) : (
              visibleLogs.map((l, idx) => (
                <HonoLogRow
                  key={`${l.timestamp}-${idx}`}
                  entry={l}
                  onSelectSession={(sid) => {
                    setCategoryFilter('session')
                    setSessionFilter(sid)
                  }}
                />
              ))
            )}
          </CompactScrollArea>
        </div>
      )}
    </>
  )

  if (embedded) {
    return <div className="flex flex-col gap-5">{opsSections}</div>
  }

  return (
    <ScrollPage smoothScroll={smoothScroll}>
      <div className="mx-auto flex w-full max-w-[960px] flex-col gap-6">
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-[22px] font-semibold tracking-[-0.03em]" style={{ fontFamily: 'var(--font-display)' }}>
              Dashboard
            </h1>
            <p className="mt-1 text-[13px] text-[var(--muted)]">
              Persisted, secret-scrubbed logs indexed in SQLite and rotated into categorized JSONL files
              {logsDir ? ` (${logsDir})` : ''}.
            </p>
          </div>
          <button
            type="button"
            onClick={() => {
              onRefresh()
              onQueryLogs({
                category: categoryFilter,
                level: levelFilter,
                sessionId: sessionFilter,
                search: searchQuery,
              })
            }}
            className="z-btn z-btn-secondary z-btn-sm"
          >
            <RefreshCw size={13} /> Refresh
          </button>
        </div>

        <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
          {[
            ['RAM (VmRSS)', `${rssMb} MB`, `PID ${telemetry?.pid ?? '-'} · ${telemetry?.active_jobs ?? 0} bg jobs`],
            ['Uptime & DB', `${telemetry?.uptime_secs ?? 0}s`, `${dbKb} KB WAL`],
            ['Today Spend', `$${(telemetry?.daily_cost_usd ?? 0).toFixed(4)}`, `Total $${(telemetry?.total_cost_usd ?? 0).toFixed(4)}`],
            ['Cache Hit', `${(telemetry?.cache_hit_pct ?? 0).toFixed(1)}%`, 'Prompt cache'],
          ].map(([label, val, sub]) => (
            <div key={label} className="z-surface-card flex flex-col gap-1">
              <span className="text-[11px] font-medium text-[var(--muted)] uppercase">{label}</span>
              <span className="font-mono text-[20px] font-semibold text-[var(--fg)]">{val}</span>
              <span className="font-mono text-[11px] text-[var(--muted)]">{sub}</span>
            </div>
          ))}
        </div>

        {opsSections}
      </div>
    </ScrollPage>
  )
}

