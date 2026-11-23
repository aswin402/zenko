import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  ArrowDownToLine,
  ArrowUpRight,
  Bot,
  ChevronDown,
  CornerDownLeft,
  GripHorizontal,
  Minus,
  RefreshCw,
  Square,
  Terminal,
  Trash2,
  X,
} from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { SPRING_PANEL } from '@/lib/ease'
import { api } from '@/lib/api'
import {
  TERMINAL_MONITOR_CONFIG,
  clampFloatingPosition,
  formatBytesCompact,
  formatElapsedCompact,
  loadSavedFloatingPosition,
  stripAnsiStateMachine,
  type FloatingPos,
} from '@/lib/terminal'
import type { BackgroundJob, JobReadChunk, SubagentRecord } from '@/lib/types'
import { cn } from '@/lib/utils'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/motion/popover'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { ZenkoTooltip } from '@/components/ui/Tooltip'
import { parseAnsi } from '@/components/assistant-ui/elements/terminal-block'

type SelectedTarget =
  | { kind: 'job'; id: string }
  | { kind: 'agent'; id: string }
  | null

const JOB_STATUS_STYLE: Record<
  BackgroundJob['status'],
  { label: string; color: string; running: boolean }
> = {
  running: { label: 'running', color: 'var(--accent)', running: true },
  completed: { label: 'done', color: 'var(--fg)', running: false },
  failed: { label: 'failed', color: 'var(--destructive)', running: false },
  stopped: { label: 'stopped', color: 'var(--muted)', running: false },
  interrupted: { label: 'interrupted', color: 'var(--muted)', running: false },
}

const AGENT_STATUS_STYLE: Record<
  SubagentRecord['status'],
  { label: string; color: string; running: boolean }
> = {
  running: { label: 'running', color: 'var(--accent)', running: true },
  completed: { label: 'done', color: 'var(--fg)', running: false },
  failed: { label: 'failed', color: 'var(--destructive)', running: false },
  killed: { label: 'stopped', color: 'var(--muted)', running: false },
  interrupted: { label: 'interrupted', color: 'var(--muted)', running: false },
}

function compactCommandLabel(job: BackgroundJob): string {
  const raw = job.label?.trim() || job.command.trim()
  return raw.length > 32 ? `${raw.slice(0, 32)}…` : raw
}

/**
 * Compact, draggable, hideable floating `zenko_task` background runner monitor
 * toggled from the Right Floating Dock. Strictly inherits active Zenko theme tokens (60/30/10).
 */
export function FloatingActivityMonitor({
  open,
  onClose,
  jobs,
  subagents,
  terminalBuffers,
  onHydrateBuffer,
  activeSessionId,
  isSessionRunning,
  popoverStyle = 'gooey',
  onStopJob,
  onClearJobOutput,
  onDeleteJob,
  onClearDoneJobs,
  onSendJobInput,
  onDemoteForeground,
  onKillAgent,
  onInspectAgent,
}: {
  open: boolean
  onClose: () => void
  jobs: BackgroundJob[]
  subagents: SubagentRecord[]
  terminalBuffers: Record<string, string>
  onHydrateBuffer: (jobId: string, content: string) => void
  activeSessionId: string | null
  isSessionRunning: boolean
  popoverStyle?: 'gooey' | 'crisp'
  onStopJob: (jobId: string) => void
  onClearJobOutput?: (jobId: string) => Promise<void> | void
  onDeleteJob?: (jobId: string) => Promise<void> | void
  onClearDoneJobs?: () => Promise<void> | void
  onSendJobInput: (jobId: string, input: string) => Promise<void>
  onDemoteForeground: () => Promise<void>
  onKillAgent: (agentId: string) => void
  onInspectAgent: (agentId: string) => void
}) {
  const reduce = useReducedMotion() ?? false
  const [minimized, setMinimized] = useState(false)
  const [selectorOpen, setSelectorOpen] = useState(false)
  const [selected, setSelected] = useState<SelectedTarget>(null)
  const [stdinText, setStdinText] = useState('')
  const [sendingStdin, setSendingStdin] = useState(false)
  const [loadingTail, setLoadingTail] = useState(false)
  const [nowMs, setNowMs] = useState(() => Date.now())

  const [pos, setPos] = useState<FloatingPos>(() =>
    loadSavedFloatingPosition(
      { width: TERMINAL_MONITOR_CONFIG.width, height: TERMINAL_MONITOR_CONFIG.height },
      { width: window.innerWidth, height: window.innerHeight },
    ),
  )

  const dragRef = useRef<{
    pointerId: number
    startClientX: number
    startClientY: number
    originX: number
    originY: number
  } | null>(null)
  const lastSeenRunningJobRef = useRef<string | null>(null)

  const recentJobs = useMemo(
    () => jobs.slice(0, TERMINAL_MONITOR_CONFIG.recentItemsLimit),
    [jobs],
  )
  const recentAgents = useMemo(
    () => subagents.slice(0, TERMINAL_MONITOR_CONFIG.recentItemsLimit),
    [subagents],
  )

  const runningJobs = useMemo(
    () => recentJobs.filter((j) => j.status === 'running'),
    [recentJobs],
  )
  const runningAgents = useMemo(
    () => recentAgents.filter((a) => a.status === 'running'),
    [recentAgents],
  )
  const activeTotal = runningJobs.length + runningAgents.length

  // Lightweight 1s elapsed timer ONLY while open and at least one process/subagent is running
  useEffect(() => {
    if (!open || activeTotal === 0) return
    setNowMs(Date.now())
    const id = window.setInterval(() => setNowMs(Date.now()), 1000)
    return () => window.clearInterval(id)
  }, [open, activeTotal])

  // Auto-clamp position on viewport resize or minimize toggle
  useEffect(() => {
    const clampToViewport = () => {
      const h = minimized
        ? TERMINAL_MONITOR_CONFIG.collapsedHeight
        : TERMINAL_MONITOR_CONFIG.height
      setPos((prev) =>
        clampFloatingPosition(
          prev,
          { width: Math.min(TERMINAL_MONITOR_CONFIG.width, window.innerWidth - 24), height: h },
          { width: window.innerWidth, height: window.innerHeight },
        ),
      )
    }
    clampToViewport()
    window.addEventListener('resize', clampToViewport)
    return () => window.removeEventListener('resize', clampToViewport)
  }, [minimized])

  // Auto-focus a newly spawned running job when panel is open and user isn't viewing another running job
  useEffect(() => {
    const newestRunning = runningJobs[0]?.id ?? null
    if (newestRunning && newestRunning !== lastSeenRunningJobRef.current) {
      lastSeenRunningJobRef.current = newestRunning
      const currentlyWatchingRunning =
        selected?.kind === 'job' && runningJobs.some((j) => j.id === selected.id)
      if (!currentlyWatchingRunning) {
        setSelected({ kind: 'job', id: newestRunning })
      }
    } else if (!selected && runningAgents[0]) {
      setSelected({ kind: 'agent', id: runningAgents[0].id })
    }
  }, [runningJobs, runningAgents, selected])

  const selectedJob =
    selected?.kind === 'job' ? recentJobs.find((j) => j.id === selected.id) ?? null : null
  const selectedAgent =
    selected?.kind === 'agent' ? recentAgents.find((a) => a.id === selected.id) ?? null : null

  const hydrateJobTail = useCallback(
    async (jobId: string) => {
      setLoadingTail(true)
      try {
        const chunk = await api<JobReadChunk>(`/api/jobs/${encodeURIComponent(jobId)}/output`)
        onHydrateBuffer(jobId, stripAnsiStateMachine(chunk.content || ''))
      } catch {
        /* ignore transient fetch failure */
      } finally {
        setLoadingTail(false)
      }
    },
    [onHydrateBuffer],
  )

  // Hydrate initial log tail once when opening a job whose buffer isn't populated yet
  useEffect(() => {
    if (!open || !selectedJob) return
    if (terminalBuffers[selectedJob.id] !== undefined) return
    hydrateJobTail(selectedJob.id)
  }, [open, selectedJob, terminalBuffers, hydrateJobTail])

  const handleDragStart = (e: React.PointerEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement
    if (target.closest('button, input, [data-no-drag="true"]')) return
    e.preventDefault()
    dragRef.current = {
      pointerId: e.pointerId,
      startClientX: e.clientX,
      startClientY: e.clientY,
      originX: pos.x,
      originY: pos.y,
    }
    e.currentTarget.setPointerCapture(e.pointerId)
  }

  const handleDragMove = (e: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current
    if (!drag || drag.pointerId !== e.pointerId) return
    const h = minimized
      ? TERMINAL_MONITOR_CONFIG.collapsedHeight
      : TERMINAL_MONITOR_CONFIG.height
    const next = clampFloatingPosition(
      {
        x: drag.originX + (e.clientX - drag.startClientX),
        y: drag.originY + (e.clientY - drag.startClientY),
      },
      { width: Math.min(TERMINAL_MONITOR_CONFIG.width, window.innerWidth - 24), height: h },
      { width: window.innerWidth, height: window.innerHeight },
    )
    setPos(next)
  }

  const handleDragEnd = (e: React.PointerEvent<HTMLDivElement>) => {
    if (dragRef.current?.pointerId !== e.pointerId) return
    dragRef.current = null
    e.currentTarget.releasePointerCapture(e.pointerId)
    try {
      window.localStorage.setItem(TERMINAL_MONITOR_CONFIG.storageKey, JSON.stringify(pos))
    } catch {
      /* ignore storage errors */
    }
  }

  const selectorTitle = selectedJob
    ? compactCommandLabel(selectedJob)
    : selectedAgent
      ? `${selectedAgent.role} · ${selectedAgent.id.slice(0, 6)}`
      : activeTotal > 0
        ? `zenko_task (${activeTotal})`
        : 'zenko_task'

  const finishedJobCount = useMemo(
    () => jobs.filter((j) => j.status !== 'running').length,
    [jobs],
  )
  const hasBufferedLogs = useMemo(
    () => Object.values(terminalBuffers).some((buf) => Boolean(buf)),
    [terminalBuffers],
  )

  const selectorDotActive = selectedJob
    ? selectedJob.status === 'running'
    : selectedAgent
      ? selectedAgent.status === 'running'
      : activeTotal > 0

  const submitStdin = async () => {
    if (!selectedJob || selectedJob.status !== 'running' || !stdinText.trim()) return
    const payload = `${stdinText}\n`
    setStdinText('')
    setSendingStdin(true)
    try {
      await onSendJobInput(selectedJob.id, payload)
    } finally {
      setSendingStdin(false)
    }
  }

  return (
    <AnimatePresence>
      {open && (
        <motion.section
          aria-label="zenko_task Background Runner"
          initial={reduce ? { opacity: 1 } : { opacity: 0, scale: 0.96, y: 10 }}
          animate={{ opacity: 1, scale: 1, y: 0 }}
          exit={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.96, y: 8 }}
          transition={SPRING_PANEL}
          style={{
            left: pos.x,
            top: pos.y,
            width: Math.min(TERMINAL_MONITOR_CONFIG.width, window.innerWidth - 24),
            height: minimized
              ? TERMINAL_MONITOR_CONFIG.collapsedHeight
              : Math.min(TERMINAL_MONITOR_CONFIG.height, window.innerHeight - 32),
          }}
          className="fixed z-40 flex flex-col overflow-hidden rounded-[20px] border border-[var(--border)] bg-[var(--surface)]/98 text-[var(--fg)] shadow-[0_20px_56px_rgba(0,0,0,0.52),inset_0_1px_0_rgba(255,255,255,0.04)]"
        >
          {/* Draggable Top Bar */}
          <div
            onPointerDown={handleDragStart}
            onPointerMove={handleDragMove}
            onPointerUp={handleDragEnd}
            onPointerCancel={handleDragEnd}
            className="flex h-[46px] shrink-0 cursor-grab items-center justify-between gap-2 border-b border-[var(--border)]/80 px-3 select-none active:cursor-grabbing"
          >
            <div className="flex min-w-0 items-center gap-2">
              <Popover
                open={selectorOpen}
                onOpenChange={setSelectorOpen}
                side="bottom"
                align="start"
                sideOffset={8}
                panelRadius={14}
                gooStrength={popoverStyle === 'gooey' ? 7 : 0}
              >
                <PopoverTrigger>
                  <button
                    type="button"
                    data-no-drag="true"
                    className="inline-flex max-w-[205px] cursor-pointer items-center gap-1.5 rounded-full border border-[var(--border)] bg-[var(--card)] px-2.5 py-1 text-[11.5px] font-medium text-[var(--fg)] transition-colors hover:border-[var(--accent)]/50"
                  >
                    <Terminal
                      size={12}
                      className="shrink-0"
                      style={{ color: selectorDotActive ? 'var(--accent)' : 'var(--muted)' }}
                    />
                    <span className="truncate font-mono text-[11px]">{selectorTitle}</span>
                    <ChevronDown size={11.5} className="shrink-0 text-[var(--muted)]" />
                  </button>
                </PopoverTrigger>
                <PopoverContent className="w-[300px] rounded-[14px] border border-[var(--border)] bg-[var(--surface)] p-1.5 shadow-2xl">
                  <CompactScrollArea autoHeight viewportClassName="max-h-72 pr-2">
                    <div className="flex flex-col gap-1">
                      <button
                        type="button"
                        onClick={() => {
                          setSelected(null)
                          setSelectorOpen(false)
                        }}
                        className={cn(
                          'flex w-full cursor-pointer items-center justify-between rounded-lg px-2.5 py-1.5 text-left text-[11.5px] transition-colors hover:bg-[var(--card)]',
                          !selected && 'bg-[var(--card)] font-medium',
                        )}
                      >
                        <span>Overview</span>
                        <span className="font-mono text-[10px] text-[var(--muted)]">
                          {activeTotal} running
                        </span>
                      </button>

                      {recentJobs.length > 0 && (
                        <div className="mt-1 px-2 font-mono text-[9.5px] uppercase tracking-wider text-[var(--muted)]">
                          Commands &amp; BG Tasks ({recentJobs.length})
                        </div>
                      )}
                      {recentJobs.map((job) => {
                        const st = JOB_STATUS_STYLE[job.status]
                        const isPicked = selected?.kind === 'job' && selected.id === job.id
                        return (
                          <button
                            key={job.id}
                            type="button"
                            onClick={() => {
                              setSelected({ kind: 'job', id: job.id })
                              setMinimized(false)
                              setSelectorOpen(false)
                            }}
                            className={cn(
                              'flex w-full cursor-pointer items-center justify-between gap-2 rounded-lg px-2.5 py-1.5 text-left transition-colors hover:bg-[var(--card)]',
                              isPicked && 'bg-[var(--card)]',
                            )}
                          >
                            <span className="flex min-w-0 items-center gap-1.5">
                              <span
                                className={cn(
                                  'h-1.5 w-1.5 shrink-0 rounded-full',
                                  st.running && 'animate-pulse',
                                )}
                                style={{ backgroundColor: st.color }}
                              />
                              <span className="truncate font-mono text-[11px] text-[var(--fg)]">
                                {compactCommandLabel(job)}
                              </span>
                            </span>
                            <span className="shrink-0 font-mono text-[10px] text-[var(--muted)]">
                              {st.label}
                            </span>
                          </button>
                        )
                      })}

                      {recentAgents.length > 0 && (
                        <div className="mt-1 px-2 font-mono text-[9.5px] uppercase tracking-wider text-[var(--muted)]">
                          Subagents ({recentAgents.length})
                        </div>
                      )}
                      {recentAgents.map((ag) => {
                        const st = AGENT_STATUS_STYLE[ag.status]
                        const isPicked = selected?.kind === 'agent' && selected.id === ag.id
                        return (
                          <button
                            key={ag.id}
                            type="button"
                            onClick={() => {
                              setSelected({ kind: 'agent', id: ag.id })
                              setMinimized(false)
                              setSelectorOpen(false)
                            }}
                            className={cn(
                              'flex w-full cursor-pointer items-center justify-between gap-2 rounded-lg px-2.5 py-1.5 text-left transition-colors hover:bg-[var(--card)]',
                              isPicked && 'bg-[var(--card)]',
                            )}
                          >
                            <span className="flex min-w-0 items-center gap-1.5">
                              <Bot size={11.5} style={{ color: st.color }} className="shrink-0" />
                              <span className="truncate font-mono text-[11px] text-[var(--fg)]">
                                {ag.role}: {ag.goal.slice(0, 26)}
                              </span>
                            </span>
                            <span className="shrink-0 font-mono text-[10px] text-[var(--muted)]">
                              {st.label}
                            </span>
                          </button>
                        )
                      })}
                    </div>
                  </CompactScrollArea>
                </PopoverContent>
              </Popover>

              <span className="font-mono text-[10.5px] tabular-nums text-[var(--muted)]">
                {activeTotal > 0 ? `${activeTotal} live` : 'idle'}
              </span>
            </div>

            <div className="flex shrink-0 items-center gap-1">
              {(finishedJobCount > 0 || hasBufferedLogs) && onClearDoneJobs && !selectedJob && (
                <ZenkoTooltip label="Clear finished tasks & terminal logs" side="bottom">
                  <button
                    type="button"
                    data-no-drag="true"
                    onClick={async () => {
                      await onClearDoneJobs()
                      setSelected((prev) => {
                        if (prev?.kind === 'job') {
                          const stillRunning = runningJobs.some((j) => j.id === prev.id)
                          return stillRunning ? prev : null
                        }
                        return prev
                      })
                    }}
                    className="inline-flex h-6 cursor-pointer items-center gap-1 rounded-md px-1.5 font-mono text-[10px] text-[var(--muted)] transition-colors hover:bg-[var(--card)] hover:text-[var(--fg)]"
                  >
                    <Trash2 size={11} />
                    <span>Clear</span>
                  </button>
                </ZenkoTooltip>
              )}
              <GripHorizontal size={13} className="mr-0.5 text-[var(--muted)]/55" />
              <ZenkoTooltip label={minimized ? 'Expand' : 'Minimize'} side="bottom">
                <button
                  type="button"
                  data-no-drag="true"
                  onClick={() => setMinimized((m) => !m)}
                  aria-label={minimized ? 'Expand zenko_task' : 'Minimize zenko_task'}
                  className="flex h-6 w-6 cursor-pointer items-center justify-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--card)] hover:text-[var(--fg)]"
                >
                  <Minus size={13} />
                </button>
              </ZenkoTooltip>
              <ZenkoTooltip label="Hide zenko_task" side="bottom">
                <button
                  type="button"
                  data-no-drag="true"
                  onClick={onClose}
                  aria-label="Hide zenko_task"
                  className="flex h-6 w-6 cursor-pointer items-center justify-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--card)] hover:text-[var(--fg)]"
                >
                  <X size={13.5} />
                </button>
              </ZenkoTooltip>
            </div>
          </div>

          {/* Panel Body */}
          {!minimized && (
            <div className="flex min-h-0 flex-1 flex-col">
              {selectedJob ? (
                <>
                  {/* Selected Command Header */}
                  <div className="flex flex-col gap-1.5 border-b border-[var(--border)]/70 bg-[var(--card)]/55 px-3.5 py-2.5">
                    <div className="flex items-center justify-between gap-2">
                      <div className="flex min-w-0 items-center gap-1.5 font-mono text-[10.5px]">
                        <span
                          className={cn(
                            'h-1.5 w-1.5 shrink-0 rounded-full',
                            selectedJob.status === 'running' && 'animate-pulse',
                          )}
                          style={{ backgroundColor: JOB_STATUS_STYLE[selectedJob.status].color }}
                        />
                        <span className="uppercase text-[var(--fg)]">
                          {JOB_STATUS_STYLE[selectedJob.status].label}
                        </span>
                        <span className="rounded border border-[var(--border)] bg-[var(--bg)] px-1 py-px text-[9.5px] uppercase text-[var(--muted)]">
                          {selectedJob.kind || 'shell'}
                        </span>
                        <span className="tabular-nums text-[var(--muted)]">
                          {formatElapsedCompact(selectedJob.started_at, selectedJob.finished_at, nowMs)}
                        </span>
                        <span className="text-[var(--muted)]">
                          · {formatBytesCompact(selectedJob.bytes_written)}
                        </span>
                        {selectedJob.exit_code != null && (
                          <span className="text-[var(--muted)]">· exit {selectedJob.exit_code}</span>
                        )}
                      </div>

                      <div className="flex shrink-0 items-center gap-1">
                        <ZenkoTooltip label="Reload tail output" side="bottom">
                          <button
                            type="button"
                            onClick={() => hydrateJobTail(selectedJob.id)}
                            className="flex h-6 w-6 cursor-pointer items-center justify-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--bg)] hover:text-[var(--fg)]"
                          >
                            <RefreshCw size={11.5} className={loadingTail ? 'animate-spin' : ''} />
                          </button>
                        </ZenkoTooltip>
                        {onClearJobOutput && (
                          <ZenkoTooltip label="Clear task log output" side="bottom">
                            <button
                              type="button"
                              onClick={() => onClearJobOutput(selectedJob.id)}
                              className="inline-flex h-6 cursor-pointer items-center gap-1 rounded-md border border-[var(--border)] bg-[var(--bg)] px-1.5 font-mono text-[10px] text-[var(--muted)] transition-colors hover:border-[var(--accent)]/45 hover:text-[var(--fg)]"
                            >
                              <Trash2 size={10} />
                              <span>Clear</span>
                            </button>
                          </ZenkoTooltip>
                        )}
                        {selectedJob.status === 'running' &&
                          isSessionRunning &&
                          activeSessionId &&
                          selectedJob.session_id === activeSessionId && (
                            <ZenkoTooltip label="Hand off running command to background" side="bottom">
                              <button
                                type="button"
                                onClick={onDemoteForeground}
                                className="inline-flex h-6 cursor-pointer items-center gap-1 rounded-md border border-[var(--border)] bg-[var(--bg)] px-1.5 font-mono text-[10px] text-[var(--muted)] transition-colors hover:border-[var(--accent)]/45 hover:text-[var(--fg)]"
                              >
                                <ArrowDownToLine size={10.5} />
                                <span>Bg</span>
                              </button>
                            </ZenkoTooltip>
                          )}
                        {selectedJob.status === 'running' && (
                          <ZenkoTooltip label="Stop background task" side="bottom">
                            <button
                              type="button"
                              onClick={() => onStopJob(selectedJob.id)}
                              className="inline-flex h-6 cursor-pointer items-center gap-1 rounded-md border border-[var(--destructive)]/40 bg-[var(--destructive)]/10 px-1.5 font-mono text-[10px] text-[var(--destructive)] transition-colors hover:bg-[var(--destructive)]/20"
                            >
                              <Square size={9} fill="currentColor" />
                              <span>Stop</span>
                            </button>
                          </ZenkoTooltip>
                        )}
                      </div>
                    </div>

                    <div
                      className="truncate font-mono text-[11px] text-[var(--muted)]"
                      title={selectedJob.command}
                    >
                      $ {selectedJob.command}
                    </div>
                  </div>

                  {/* Live Terminal Output Stream (ANSI-aware via Assistant-UI TerminalBlock parser) */}
                  <CompactScrollArea
                    stickToBottom
                    className="min-h-0 flex-1 bg-[var(--bg)]"
                    viewportClassName="p-3 pr-4"
                  >
                    <div className="flex flex-col gap-0.5 font-mono text-[11px] leading-[1.55] text-[var(--fg)]/90">
                      {(terminalBuffers[selectedJob.id]
                        ? terminalBuffers[selectedJob.id].split('\n')
                        : [
                            loadingTail
                              ? 'Loading terminal stream...'
                              : selectedJob.status === 'running'
                                ? 'Waiting for output...'
                                : '(no output)',
                          ]
                      ).map((line, lineIdx) => (
                        <div key={lineIdx} className="break-words whitespace-pre-wrap">
                          {parseAnsi(line).map((seg, segIdx) => (
                            <span
                              key={segIdx}
                              className={cn(seg.bold && 'font-semibold', seg.dim && 'opacity-60')}
                            >
                              {seg.text}
                            </span>
                          ))}
                          {line === '' ? ' ' : null}
                        </div>
                      ))}
                      {selectedJob.status === 'running' && (
                        <span
                          aria-hidden
                          className="mt-0.5 inline-block h-3 w-1.5 animate-pulse bg-[var(--accent)]/80 motion-reduce:animate-none"
                        />
                      )}
                    </div>
                  </CompactScrollArea>

                  {/* Interactive Stdin Line (running processes only) */}
                  {selectedJob.status === 'running' && (
                    <div className="flex items-center gap-2 border-t border-[var(--border)]/80 bg-[var(--surface)] px-3 py-2">
                      <span className="font-mono text-[11px] text-[var(--accent)]">&gt;</span>
                      <input
                        value={stdinText}
                        onChange={(e) => setStdinText(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === 'Enter') {
                            e.preventDefault()
                            submitStdin()
                          }
                        }}
                        disabled={sendingStdin}
                        placeholder="Send stdin line..."
                        className="min-w-0 flex-1 bg-transparent font-mono text-[11.5px] text-[var(--fg)] placeholder:text-[var(--muted)]/60 focus:outline-none"
                      />
                      <button
                        type="button"
                        disabled={sendingStdin || !stdinText.trim()}
                        onClick={submitStdin}
                        aria-label="Send stdin"
                        className="flex h-6 w-6 cursor-pointer items-center justify-center rounded-md border border-[var(--border)] bg-[var(--card)] text-[var(--muted)] transition-colors hover:border-[var(--accent)]/50 hover:text-[var(--fg)] disabled:opacity-40"
                      >
                        <CornerDownLeft size={11.5} />
                      </button>
                    </div>
                  )}
                </>
              ) : selectedAgent ? (
                <>
                  <div className="flex flex-col gap-1.5 border-b border-[var(--border)]/70 bg-[var(--card)]/55 px-3.5 py-2.5">
                    <div className="flex items-center justify-between gap-2">
                      <div className="flex min-w-0 items-center gap-1.5 font-mono text-[10.5px]">
                        <Bot
                          size={12}
                          style={{ color: AGENT_STATUS_STYLE[selectedAgent.status].color }}
                        />
                        <span className="uppercase text-[var(--fg)]">{selectedAgent.role}</span>
                        <span className="text-[var(--muted)]">
                          · {AGENT_STATUS_STYLE[selectedAgent.status].label}
                        </span>
                        <span className="tabular-nums text-[var(--muted)]">
                          · {formatElapsedCompact(selectedAgent.started_at, selectedAgent.finished_at, nowMs)}
                        </span>
                      </div>
                      <div className="flex shrink-0 items-center gap-1">
                        <button
                          type="button"
                          onClick={() => onInspectAgent(selectedAgent.id)}
                          className="inline-flex h-6 cursor-pointer items-center gap-1 rounded-md border border-[var(--border)] bg-[var(--bg)] px-2 font-mono text-[10px] text-[var(--fg)] transition-colors hover:border-[var(--accent)]/50"
                        >
                          <span>Inspect</span>
                          <ArrowUpRight size={10.5} />
                        </button>
                        {selectedAgent.status === 'running' && (
                          <button
                            type="button"
                            onClick={() => onKillAgent(selectedAgent.id)}
                            className="inline-flex h-6 cursor-pointer items-center gap-1 rounded-md border border-[var(--destructive)]/40 bg-[var(--destructive)]/10 px-1.5 font-mono text-[10px] text-[var(--destructive)]"
                          >
                            <Square size={9} fill="currentColor" />
                            <span>Stop</span>
                          </button>
                        )}
                      </div>
                    </div>
                    <p className="line-clamp-2 text-[11.5px] text-[var(--muted)]">
                      {selectedAgent.goal}
                    </p>
                  </div>

                  <CompactScrollArea
                    className="min-h-0 flex-1 bg-[var(--bg)]"
                    viewportClassName="p-3 pr-4"
                  >
                    <pre className="whitespace-pre-wrap break-words font-mono text-[11px] leading-relaxed text-[var(--fg)]/88">
                      {selectedAgent.structured_output
                        ? JSON.stringify(selectedAgent.structured_output, null, 2)
                        : selectedAgent.summary ||
                          (selectedAgent.status === 'running'
                            ? 'Subagent working autonomously...'
                            : '(no summary recorded)')}
                    </pre>
                  </CompactScrollArea>
                </>
              ) : (
                /* Clean Compact Overview for zenko_task */
                <CompactScrollArea className="min-h-0 flex-1" viewportClassName="p-4 pr-5">
                  <div className="flex flex-col items-center py-4 text-center">
                    <div className="mb-3 flex h-11 w-11 items-center justify-center rounded-2xl border border-[var(--border)] bg-[var(--card)] shadow-inner">
                      <Terminal
                        size={20}
                        style={{ color: activeTotal > 0 ? 'var(--accent)' : 'var(--fg)' }}
                      />
                    </div>
                    <h3 className="font-mono text-[15px] font-semibold tracking-tight text-[var(--fg)]">
                      zenko_task
                    </h3>
                    <p className="mt-1 max-w-[280px] text-[12px] leading-relaxed text-[var(--muted)]">
                      Live background runner — watch cargo build, dev servers, tests, and subagents in real time.
                    </p>
                  </div>

                  {(recentJobs.length > 0 || recentAgents.length > 0) && (
                    <div className="mb-2 flex items-center justify-between px-0.5 font-mono text-[10px] uppercase tracking-wider text-[var(--muted)]">
                      <span>Recent Tasks ({recentJobs.length})</span>
                      {finishedJobCount > 0 && onClearDoneJobs && (
                        <button
                          type="button"
                          onClick={() => onClearDoneJobs()}
                          className="inline-flex cursor-pointer items-center gap-1 rounded px-1.5 py-0.5 normal-case text-[10.5px] text-[var(--muted)] transition-colors hover:bg-[var(--card)] hover:text-[var(--accent)]"
                        >
                          <Trash2 size={10.5} />
                          <span>Clear Logs ({finishedJobCount})</span>
                        </button>
                      )}
                    </div>
                  )}

                  <div className="flex flex-col gap-2">
                    {recentJobs.slice(0, 6).map((job) => {
                      const st = JOB_STATUS_STYLE[job.status]
                      return (
                        <div
                          key={job.id}
                          className="z-card-item group flex w-full items-center gap-2 px-3 py-2.5 transition-colors hover:border-[var(--accent)]/45"
                        >
                          <button
                            type="button"
                            onClick={() => setSelected({ kind: 'job', id: job.id })}
                            className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 text-left"
                          >
                            <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--bg)]">
                              <Terminal size={13} style={{ color: st.color }} />
                            </div>
                            <div className="min-w-0 flex-1">
                              <div className="flex items-center justify-between gap-2">
                                <span className="truncate font-mono text-[11.5px] font-medium text-[var(--fg)]">
                                  {compactCommandLabel(job)}
                                </span>
                                <span
                                  className="shrink-0 font-mono text-[10px] uppercase"
                                  style={{ color: st.color }}
                                >
                                  {st.label}
                                </span>
                              </div>
                              <div className="mt-0.5 flex items-center gap-2 font-mono text-[10.5px] text-[var(--muted)]">
                                <span>{job.kind || 'shell'}</span>
                                <span>·</span>
                                <span>{formatBytesCompact(job.bytes_written)}</span>
                                <span>·</span>
                                <span className="tabular-nums">
                                  {formatElapsedCompact(job.started_at, job.finished_at, nowMs)}
                                </span>
                              </div>
                            </div>
                          </button>
                          {job.status !== 'running' && onDeleteJob && (
                            <ZenkoTooltip label="Delete task & log" side="left">
                              <button
                                type="button"
                                aria-label="Delete task"
                                onClick={() => onDeleteJob(job.id)}
                                className="flex h-6 w-6 shrink-0 cursor-pointer items-center justify-center rounded-md text-[var(--muted)]/60 opacity-0 transition-all hover:bg-[var(--bg)] hover:text-[var(--destructive)] group-hover:opacity-100"
                              >
                                <Trash2 size={11.5} />
                              </button>
                            </ZenkoTooltip>
                          )}
                        </div>
                      )
                    })}

                    {recentAgents.slice(0, 2).map((ag) => {
                      const st = AGENT_STATUS_STYLE[ag.status]
                      return (
                        <button
                          key={ag.id}
                          type="button"
                          onClick={() => setSelected({ kind: 'agent', id: ag.id })}
                          className="z-card-item flex w-full cursor-pointer items-center gap-3 px-3 py-2.5 text-left transition-colors hover:border-[var(--accent)]/45"
                        >
                          <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--bg)]">
                            <Bot size={13} style={{ color: st.color }} />
                          </div>
                          <div className="min-w-0 flex-1">
                            <div className="flex items-center justify-between gap-2">
                              <span className="truncate font-mono text-[11.5px] font-medium text-[var(--fg)]">
                                {ag.role} subagent
                              </span>
                              <span
                                className="shrink-0 font-mono text-[10px] uppercase"
                                style={{ color: st.color }}
                              >
                                {st.label}
                              </span>
                            </div>
                            <p className="mt-0.5 truncate text-[11px] text-[var(--muted)]">
                              {ag.goal}
                            </p>
                          </div>
                        </button>
                      )
                    })}

                    {recentJobs.length === 0 && recentAgents.length === 0 && (
                      <div className="rounded-xl border border-dashed border-[var(--border)] px-3.5 py-4 text-center font-mono text-[11px] text-[var(--muted)]">
                        No background builds, dev servers, or tasks yet.
                      </div>
                    )}
                  </div>
                </CompactScrollArea>
              )}
            </div>
          )}
        </motion.section>
      )}
    </AnimatePresence>
  )
}
