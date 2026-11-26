import { useEffect, useMemo, useState } from 'react'
import { Check, FileCode2, History, ListChecks, RotateCcw, ShieldCheck, X } from 'lucide-react'
import { api } from '@/lib/api'
import { Alert, AlertContent, AlertDescription, AlertIcon } from '@/components/motion/alert'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/motion/popover'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { ZenkoTooltip } from '@/components/ui/Tooltip'
import { cn } from '@/lib/utils'
import type {
  CheckpointDiffReport,
  PermissionMode,
  RewindMode,
  SessionTask,
  SessionTaskStatus,
  WorkspaceCheckpoint,
} from '@/lib/types'

const MAX_VISIBLE_CHECKPOINTS = 12
const APPROVE_AND_EXECUTE_PROMPT =
  'The plan is approved. Execute the tracked Task DAG step by step and verify each change.'

const TASK_STATUS_STYLE: Record<SessionTaskStatus, { label: string; color: string }> = {
  pending: { label: 'pending', color: 'var(--muted)' },
  in_progress: { label: 'active', color: 'var(--accent)' },
  blocked: { label: 'blocked', color: 'var(--muted)' },
  done: { label: 'done', color: 'var(--fg)' },
  failed: { label: 'failed', color: 'var(--destructive)' },
}

const REWIND_MODE_OPTIONS: ReadonlyArray<readonly [RewindMode, string]> = [
  ['code_and_conversation', 'Code + Chat'],
  ['code_only', 'Code'],
  ['conversation_only', 'Chat'],
]

function formatCheckpointLabel(raw: string): string {
  const trimmed = raw.trim()
  return trimmed.startsWith('Before: ') ? trimmed.slice('Before: '.length) : trimmed
}

/**
 * Top banner shown ONLY when the active session is in Read-Only Plan Mode.
 * Normal sessions keep a clean, zero-chrome chat canvas.
 */
export function Phase6Bar({
  permissionMode,
  onApprovePlan,
}: {
  permissionMode: PermissionMode
  onApprovePlan: (targetMode: 'balanced' | 'autonomous', executePrompt?: string) => Promise<void>
}) {
  const [approving, setApproving] = useState(false)

  if (permissionMode !== 'plan') {
    return null
  }

  return (
    <div className="border-b border-[var(--border)] bg-[var(--surface)]/92 px-5 py-2">
      <div className="mx-auto flex max-w-[920px] flex-wrap items-center justify-between gap-2">
        <span className="inline-flex items-center gap-1.5 rounded-[var(--radius-sm)] border border-[var(--accent)]/45 bg-[var(--bg)] px-2 py-0.5 font-mono text-[11px] text-[var(--accent)]">
          <ShieldCheck size={12} />
          PLAN MODE · READ-ONLY
        </span>

        <div className="flex items-center gap-1.5">
          <button
            type="button"
            disabled={approving}
            onClick={async () => {
              setApproving(true)
              try {
                await onApprovePlan('balanced')
              } finally {
                setApproving(false)
              }
            }}
            className="z-btn z-btn-secondary z-btn-sm font-mono text-[11px]"
          >
            Approve (Balanced)
          </button>
          <button
            type="button"
            disabled={approving}
            onClick={async () => {
              setApproving(true)
              try {
                await onApprovePlan('autonomous', APPROVE_AND_EXECUTE_PROMPT)
              } finally {
                setApproving(false)
              }
            }}
            className="z-btn z-btn-primary z-btn-sm font-mono text-[11px]"
          >
            <Check size={12} />
            Approve & Execute
          </button>
        </div>
      </div>
    </div>
  )
}

/**
 * Minimal, theme-native Checkpoint & Rewind action rendered at the end of assistant messages
 * (right beside Copy & Retry), keeping the chat composer toolbar uncluttered on one row.
 */
export function CheckpointMenuButton({
  sessionId,
  checkpoints,
  turnCheckpointId = null,
  popoverStyle = 'gooey',
  showCount = false,
  minimalTrigger = false,
  onRewind,
}: {
  sessionId: string | null
  checkpoints: WorkspaceCheckpoint[]
  turnCheckpointId?: string | null
  popoverStyle?: 'gooey' | 'crisp'
  showCount?: boolean
  minimalTrigger?: boolean
  onRewind: (
    checkpointId: string,
    mode: RewindMode,
    overwriteManual: boolean,
  ) => Promise<void>
}) {
  const [open, setOpen] = useState(false)
  const [selectedCpId, setSelectedCpId] = useState<string | null>(null)
  const [diffReport, setDiffReport] = useState<CheckpointDiffReport | null>(null)
  const [loadingDiff, setLoadingDiff] = useState(false)
  const [rewindMode, setRewindMode] = useState<RewindMode>('code_and_conversation')
  const [overwriteManual, setOverwriteManual] = useState(false)
  const [rewinding, setRewinding] = useState(false)

  useEffect(() => {
    setOpen(false)
    setSelectedCpId(null)
    setDiffReport(null)
  }, [sessionId])

  const orderedCheckpoints = useMemo(() => {
    const sliced = checkpoints.slice(0, MAX_VISIBLE_CHECKPOINTS)
    if (!turnCheckpointId) return sliced
    const idx = sliced.findIndex((c) => c.id === turnCheckpointId)
    if (idx <= 0) return sliced
    return [sliced[idx], ...sliced.slice(0, idx), ...sliced.slice(idx + 1)]
  }, [checkpoints, turnCheckpointId])

  if (orderedCheckpoints.length === 0) {
    return null
  }

  const inspectCheckpoint = async (cpId: string) => {
    if (selectedCpId === cpId) {
      setSelectedCpId(null)
      setDiffReport(null)
      return
    }
    setSelectedCpId(cpId)
    setLoadingDiff(true)
    try {
      const rep = await api<CheckpointDiffReport>(
        `/api/checkpoints/${encodeURIComponent(cpId)}/diff`,
      )
      setDiffReport(rep)
    } catch {
      setDiffReport(null)
    } finally {
      setLoadingDiff(false)
    }
  }

  const tooltipLabel = turnCheckpointId
    ? 'Rewind to checkpoint before this turn'
    : `Checkpoints (${checkpoints.length})`

  return (
    <ZenkoTooltip label={tooltipLabel} side="top" disabled={open}>
      <Popover
        open={open}
        onOpenChange={(next) => {
          setOpen(next)
          if (!next) {
            setSelectedCpId(null)
            setDiffReport(null)
          }
        }}
        side="top"
        align="start"
        sideOffset={8}
        panelRadius={12}
        gooStrength={popoverStyle === 'gooey' ? 6 : 0}
      >
        <PopoverTrigger>
          <button
            type="button"
            aria-label={tooltipLabel}
            aria-expanded={open}
            className={cn(
              minimalTrigger
                ? 'inline-flex cursor-pointer items-center gap-1 text-[11.5px] text-[var(--muted)]/80 transition-colors hover:text-[var(--fg)]'
                : 'inline-flex h-7 cursor-pointer items-center justify-center gap-1 rounded-md px-1.5 text-muted-foreground outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring',
              !minimalTrigger && !showCount && 'w-7 px-0',
              open && 'bg-[var(--card)] text-[var(--accent)]',
            )}
          >
            <History className="size-3.5" />
            {minimalTrigger && <span>Rewind</span>}
            {showCount && checkpoints.length > 0 && (
              <span className="font-mono text-[10px] tabular-nums opacity-80">
                {checkpoints.length}
              </span>
            )}
          </button>
        </PopoverTrigger>

        <PopoverContent className="w-[376px] max-w-[90vw] rounded-[12px] border border-[var(--border)] bg-[var(--surface)] p-2.5 shadow-2xl">
          {/* Minimal 1-line Header */}
          <div className="mb-2 flex items-center justify-between gap-2">
            <div className="flex items-center gap-1.5">
              <History size={12.5} style={{ color: 'var(--accent)' }} />
              <span className="text-[12px] font-medium text-[var(--fg)]">Checkpoints</span>
              <span className="rounded-full border border-[var(--border)] bg-[var(--bg)] px-1.5 py-px font-mono text-[9.5px] tabular-nums text-[var(--muted)]">
                {checkpoints.length}
              </span>
            </div>
            <button
              type="button"
              aria-label="Close checkpoints"
              onClick={() => setOpen(false)}
              className="grid size-6 cursor-pointer place-items-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--card)] hover:text-[var(--fg)]"
            >
              <X size={12} />
            </button>
          </div>

          {/* Recessed Segmented Mode Switcher + Theme-Native Force Toggle */}
          <div className="mb-2 flex items-center justify-between gap-2">
            <div
              role="radiogroup"
              aria-label="Rewind scope"
              className="inline-flex items-center rounded-lg border border-[var(--border)] bg-[var(--bg)] p-0.5"
            >
              {REWIND_MODE_OPTIONS.map(([m, label]) => {
                const active = rewindMode === m
                return (
                  <button
                    key={m}
                    type="button"
                    role="radio"
                    aria-checked={active}
                    onClick={() => setRewindMode(m)}
                    className={cn(
                      'cursor-pointer rounded-[6px] px-2 py-0.5 font-mono text-[10px] transition-colors',
                      active
                        ? 'bg-[var(--surface)] font-medium text-[var(--accent)] shadow-xs'
                        : 'text-[var(--muted)] hover:text-[var(--fg)]',
                    )}
                  >
                    {label}
                  </button>
                )
              })}
            </div>

            <ZenkoTooltip label="Overwrite your manual file edits when rewinding" side="top">
              <button
                type="button"
                role="checkbox"
                aria-checked={overwriteManual}
                onClick={() => setOverwriteManual((v) => !v)}
                className="inline-flex cursor-pointer items-center gap-1.5 rounded-md px-1.5 py-1 font-mono text-[10px] text-[var(--muted)] transition-colors hover:text-[var(--fg)]"
              >
                <span
                  aria-hidden="true"
                  className={cn(
                    'flex h-3.5 w-3.5 items-center justify-center rounded-[4px] border transition-colors',
                    overwriteManual
                      ? 'border-[var(--accent)] bg-[var(--accent)] text-[var(--primary-fg)]'
                      : 'border-[var(--border)] bg-[var(--bg)]',
                  )}
                >
                  {overwriteManual && <Check size={9.5} strokeWidth={3} />}
                </span>
                <span>Force</span>
              </button>
            </ZenkoTooltip>
          </div>

          {/* Minimal Checkpoint List */}
          <CompactScrollArea autoHeight viewportClassName="max-h-60 pr-2">
            <div className="flex flex-col gap-1">
              {orderedCheckpoints.map((cp) => {
                const isOpen = selectedCpId === cp.id
                const isThisTurn = turnCheckpointId === cp.id
                return (
                  <div
                    key={cp.id}
                    className={cn(
                      'rounded-lg border bg-[var(--bg)]/85 px-2.5 py-1.5 transition-colors',
                      isThisTurn
                        ? 'border-[var(--accent)]/40'
                        : 'border-[var(--border)]/70 hover:border-[var(--border)]',
                    )}
                  >
                    <div className="flex items-center justify-between gap-2 text-[11px]">
                      <button
                        type="button"
                        onClick={() => inspectCheckpoint(cp.id)}
                        className="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 text-left"
                      >
                        <span className="truncate font-medium text-[var(--fg)]/90">
                          {formatCheckpointLabel(cp.label)}
                        </span>
                        {isThisTurn && (
                          <span className="shrink-0 font-mono text-[9px] text-[var(--accent)]">
                            turn
                          </span>
                        )}
                        {cp.is_safety_backup && (
                          <span className="shrink-0 rounded border border-[var(--border)] px-1 font-mono text-[9px] text-[var(--muted)]">
                            backup
                          </span>
                        )}
                        <span className="shrink-0 font-mono text-[9.5px] tabular-nums text-[var(--muted)]/80">
                          {cp.changed_files.length}f
                        </span>
                      </button>

                      <div className="flex shrink-0 items-center gap-1">
                        <ZenkoTooltip label={isOpen ? 'Hide diff' : 'Inspect diff'} side="top">
                          <button
                            type="button"
                            aria-label={isOpen ? 'Hide checkpoint diff' : 'Inspect checkpoint diff'}
                            onClick={() => inspectCheckpoint(cp.id)}
                            className={cn(
                              'inline-flex h-6 cursor-pointer items-center gap-1 rounded-md border px-1.5 font-mono text-[10px] transition-colors',
                              isOpen
                                ? 'border-[var(--accent)]/45 bg-[var(--surface)] text-[var(--accent)]'
                                : 'border-transparent text-[var(--muted)] hover:border-[var(--border)] hover:bg-[var(--surface)] hover:text-[var(--fg)]',
                            )}
                          >
                            <FileCode2 size={10.5} />
                            <span>{isOpen ? 'Hide' : 'Diff'}</span>
                          </button>
                        </ZenkoTooltip>
                        <button
                          type="button"
                          disabled={rewinding}
                          onClick={async () => {
                            setRewinding(true)
                            try {
                              await onRewind(cp.id, rewindMode, overwriteManual)
                              setSelectedCpId(null)
                              setDiffReport(null)
                              setOpen(false)
                            } finally {
                              setRewinding(false)
                            }
                          }}
                          className="inline-flex h-6 cursor-pointer items-center gap-1 rounded-md border border-[var(--border)] bg-[var(--surface)] px-2 font-mono text-[10px] text-[var(--fg)] transition-colors hover:border-[var(--accent)]/55 hover:text-[var(--accent)] disabled:opacity-45"
                        >
                          <RotateCcw size={10} />
                          <span>Rewind</span>
                        </button>
                      </div>
                    </div>

                    {isOpen && (
                      <div className="mt-2 border-t border-[var(--border)]/60 pt-2 font-mono text-[10px]">
                        {loadingDiff ? (
                          <div className="text-[var(--muted)]">Computing diff...</div>
                        ) : diffReport ? (
                          <>
                            {diffReport.manual_edit_conflicts.length > 0 && (
                              <Alert variant="warning" className="mb-2 px-2 py-1.5 text-[10px]">
                                <AlertIcon />
                                <AlertContent>
                                  <AlertDescription className="text-[10px] text-[var(--fg)]">
                                    Manual edits kept unless Force is enabled:{' '}
                                    <span className="font-mono">
                                      {diffReport.manual_edit_conflicts.join(', ')}
                                    </span>
                                  </AlertDescription>
                                </AlertContent>
                              </Alert>
                            )}
                            {diffReport.diff_stat && (
                              <div className="mb-1 text-[var(--muted)]">{diffReport.diff_stat}</div>
                            )}
                            <CompactScrollArea
                              autoHeight
                              className="rounded-md border border-[var(--border)] bg-[var(--surface)]"
                              viewportClassName="max-h-36 p-2 pr-3"
                            >
                              <pre className="whitespace-pre-wrap text-[9.5px] leading-relaxed text-[var(--fg)]/85">
                                {diffReport.patch_preview ||
                                  '(No filesystem changes relative to this checkpoint)'}
                              </pre>
                            </CompactScrollArea>
                          </>
                        ) : (
                          <div className="text-[var(--muted)]">Diff unavailable.</div>
                        )}
                      </div>
                    )}
                  </div>
                )
              })}
            </div>
          </CompactScrollArea>
        </PopoverContent>
      </Popover>
    </ZenkoTooltip>
  )
}

/**
 * Quiet composer-toolbar BeUI Popover for Session Tasks DAG (Checkpoints now live cleanly
 * at the end of assistant messages so the chat composer stays uncluttered on one row).
 */
export function WorkspaceToolsPopovers({
  sessionId,
  tasks,
  popoverStyle = 'gooey',
}: {
  sessionId: string | null
  tasks: SessionTask[]
  popoverStyle?: 'gooey' | 'crisp'
}) {
  const [tasksOpen, setTasksOpen] = useState(false)

  useEffect(() => {
    setTasksOpen(false)
  }, [sessionId])

  if (tasks.length === 0) {
    return null
  }

  const doneCount = tasks.filter((t) => t.status === 'done').length

  return (
    <ZenkoTooltip
      label={`Tracked Task DAG (${doneCount}/${tasks.length} done)`}
      side="top"
      disabled={tasksOpen}
    >
      <Popover
        open={tasksOpen}
        onOpenChange={setTasksOpen}
        side="top"
        align="end"
        sideOffset={10}
        panelRadius={14}
        gooStrength={popoverStyle === 'gooey' ? 7 : 0}
      >
        <PopoverTrigger>
          <button
            type="button"
            aria-label={`Tracked Task DAG (${doneCount}/${tasks.length} done)`}
            className="hidden h-7 cursor-pointer items-center gap-1 rounded-full border border-[var(--border)] bg-[var(--bg)]/80 px-2 font-mono text-[10.5px] text-[var(--muted)] transition-colors hover:border-[var(--accent)]/45 hover:text-[var(--fg)] sm:inline-flex"
          >
            <ListChecks size={12} className="text-[var(--accent)]" />
            <span>
              {doneCount}/{tasks.length}
            </span>
          </button>
        </PopoverTrigger>
        <PopoverContent className="w-[380px] max-w-[90vw] rounded-[14px] border border-[var(--border)] bg-[var(--surface)] p-3 shadow-2xl">
          <div className="mb-2 flex items-center justify-between">
            <span className="flex items-center gap-1.5 text-[12px] font-medium text-[var(--fg)]">
              <ListChecks size={13} className="text-[var(--accent)]" />
              Session Tasks ({doneCount}/{tasks.length})
            </span>
            <button
              type="button"
              aria-label="Close session tasks"
              onClick={() => setTasksOpen(false)}
              className="cursor-pointer text-[var(--muted)] hover:text-[var(--fg)]"
            >
              <X size={12} />
            </button>
          </div>
          <CompactScrollArea autoHeight viewportClassName="max-h-64 pr-2.5">
            <div className="flex flex-col gap-1.5">
              {tasks.map((t) => {
                const meta = TASK_STATUS_STYLE[t.status] || TASK_STATUS_STYLE.pending
                return (
                  <div
                    key={t.id}
                    className="flex flex-wrap items-center justify-between gap-2 rounded-[var(--radius-sm)] border border-[var(--border)]/70 bg-[var(--bg)] px-2.5 py-1.5 text-[11.5px]"
                  >
                    <div className="flex min-w-0 items-center gap-2">
                      <span
                        className="shrink-0 font-mono text-[10px] uppercase"
                        style={{ color: meta.color }}
                      >
                        [{meta.label}]
                      </span>
                      <span className="truncate font-medium text-[var(--fg)]">{t.title}</span>
                    </div>
                    {t.owner_agent && (
                      <span className="rounded border border-[var(--border)] bg-[var(--surface)] px-1.5 py-px font-mono text-[9.5px] text-[var(--muted)]">
                        {t.owner_agent.slice(0, 10)}
                      </span>
                    )}
                  </div>
                )
              })}
            </div>
          </CompactScrollArea>
        </PopoverContent>
      </Popover>
    </ZenkoTooltip>
  )
}
