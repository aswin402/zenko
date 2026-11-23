import { useEffect, useState } from 'react'
import { Check, History, ListChecks, RotateCcw, ShieldCheck, X } from 'lucide-react'
import { api } from '@/lib/api'
import { Alert, AlertContent, AlertDescription, AlertIcon } from '@/components/motion/alert'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/motion/popover'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
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
  ['code_only', 'Code Only'],
  ['conversation_only', 'Chat Only'],
]

/**
 * Top banner shown ONLY when the active session is in Read-Only Plan Mode.
 * Normal sessions keep a clean, zero-chrome chat canvas; Checkpoints & Task DAG
 * live unobtrusively as BeUI popovers inside the composer toolbar (`WorkspaceToolsPopovers`).
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
 * Quiet composer-toolbar BeUI Popovers for Session Tasks DAG & Workspace Checkpoints
 * (auto-saved file undo points before agent edits). Hidden cleanly when empty.
 */
export function WorkspaceToolsPopovers({
  sessionId,
  tasks,
  checkpoints,
  popoverStyle = 'gooey',
  onRewind,
}: {
  sessionId: string | null
  tasks: SessionTask[]
  checkpoints: WorkspaceCheckpoint[]
  popoverStyle?: 'gooey' | 'crisp'
  onRewind: (
    checkpointId: string,
    mode: RewindMode,
    overwriteManual: boolean,
  ) => Promise<void>
}) {
  const [tasksOpen, setTasksOpen] = useState(false)
  const [cpOpen, setCpOpen] = useState(false)
  const [selectedCpId, setSelectedCpId] = useState<string | null>(null)
  const [diffReport, setDiffReport] = useState<CheckpointDiffReport | null>(null)
  const [loadingDiff, setLoadingDiff] = useState(false)
  const [rewindMode, setRewindMode] = useState<RewindMode>('code_and_conversation')
  const [overwriteManual, setOverwriteManual] = useState(false)
  const [rewinding, setRewinding] = useState(false)

  useEffect(() => {
    setTasksOpen(false)
    setCpOpen(false)
    setSelectedCpId(null)
    setDiffReport(null)
  }, [sessionId])

  if (tasks.length === 0 && checkpoints.length === 0) {
    return null
  }

  const doneCount = tasks.filter((t) => t.status === 'done').length
  const visibleCheckpoints = checkpoints.slice(0, MAX_VISIBLE_CHECKPOINTS)

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

  return (
    <>
      {tasks.length > 0 && (
        <Popover
          open={tasksOpen}
          onOpenChange={(next) => {
            setTasksOpen(next)
            if (next) setCpOpen(false)
          }}
          side="top"
          align="end"
          sideOffset={10}
          panelRadius={14}
          gooStrength={popoverStyle === 'gooey' ? 7 : 0}
        >
          <PopoverTrigger>
            <button
              type="button"
              title={`Tracked Task DAG (${doneCount}/${tasks.length} done)`}
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
      )}

      {visibleCheckpoints.length > 0 && (
        <Popover
          open={cpOpen}
          onOpenChange={(next) => {
            setCpOpen(next)
            if (next) setTasksOpen(false)
          }}
          side="top"
          align="end"
          sideOffset={10}
          panelRadius={14}
          gooStrength={popoverStyle === 'gooey' ? 7 : 0}
        >
          <PopoverTrigger>
            <button
              type="button"
              title="Workspace checkpoints — auto-saved undo snapshots before file edits"
              className="hidden h-7 cursor-pointer items-center gap-1 rounded-full border border-[var(--border)] bg-[var(--bg)]/80 px-2 font-mono text-[10.5px] text-[var(--muted)] transition-colors hover:border-[var(--accent)]/45 hover:text-[var(--fg)] sm:inline-flex"
            >
              <History size={12} />
              <span>{checkpoints.length}</span>
            </button>
          </PopoverTrigger>
          <PopoverContent className="w-[460px] max-w-[92vw] rounded-[14px] border border-[var(--border)] bg-[var(--surface)] p-3 shadow-2xl">
            <div className="mb-2 flex items-center justify-between">
              <div>
                <div className="flex items-center gap-1.5 text-[12px] font-medium text-[var(--fg)]">
                  <History size={13} className="text-[var(--accent)]" />
                  Workspace Checkpoints ({checkpoints.length})
                </div>
                <p className="text-[10.5px] text-[var(--muted)]">
                  Automatic undo snapshots taken before agent edits. Inspect diff or rewind safely.
                </p>
              </div>
              <button
                type="button"
                onClick={() => setCpOpen(false)}
                className="cursor-pointer text-[var(--muted)] hover:text-[var(--fg)]"
              >
                <X size={12} />
              </button>
            </div>

            <div className="mb-2 flex flex-wrap items-center justify-between gap-2 text-[10.5px]">
              <div className="flex items-center gap-1">
                {REWIND_MODE_OPTIONS.map(([m, label]) => (
                  <button
                    key={m}
                    type="button"
                    onClick={() => setRewindMode(m)}
                    className={cn(
                      'cursor-pointer rounded-[var(--radius-sm)] border px-2 py-0.5 font-mono text-[10px] transition-colors',
                      rewindMode === m
                        ? 'border-[var(--accent)] text-[var(--accent)]'
                        : 'border-[var(--border)] text-[var(--muted)] hover:text-[var(--fg)]',
                    )}
                  >
                    {label}
                  </button>
                ))}
              </div>
              <label className="flex cursor-pointer items-center gap-1.5 font-mono text-[10px] text-[var(--muted)]">
                <input
                  type="checkbox"
                  checked={overwriteManual}
                  onChange={(e) => setOverwriteManual(e.target.checked)}
                  className="accent-[var(--accent)]"
                />
                Overwrite manual edits
              </label>
            </div>

            <CompactScrollArea autoHeight viewportClassName="max-h-72 pr-2.5">
              <div className="flex flex-col gap-1.5">
              {visibleCheckpoints.map((cp) => {
                const isOpen = selectedCpId === cp.id
                return (
                  <div
                    key={cp.id}
                    className="rounded-[var(--radius-sm)] border border-[var(--border)]/80 bg-[var(--bg)] px-2.5 py-1.5"
                  >
                    <div className="flex flex-wrap items-center justify-between gap-2 text-[11.5px]">
                      <button
                        type="button"
                        onClick={() => inspectCheckpoint(cp.id)}
                        className="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 text-left"
                      >
                        <span className="font-mono text-[10px] text-[var(--muted)]">
                          {cp.id.slice(0, 8)}
                        </span>
                        <span className="truncate text-[var(--fg)]">{cp.label}</span>
                        {cp.is_safety_backup && (
                          <span className="rounded border border-[var(--border)] px-1 font-mono text-[9px] text-[var(--muted)]">
                            safety
                          </span>
                        )}
                        <span className="shrink-0 font-mono text-[9.5px] text-[var(--muted)]">
                          {cp.changed_files.length}f
                        </span>
                      </button>
                      <div className="flex items-center gap-1">
                        <button
                          type="button"
                          onClick={() => inspectCheckpoint(cp.id)}
                          className="z-btn z-btn-ghost z-btn-sm font-mono text-[10px]"
                        >
                          {isOpen ? 'Hide' : 'Diff'}
                        </button>
                        <button
                          type="button"
                          disabled={rewinding}
                          onClick={async () => {
                            setRewinding(true)
                            try {
                              await onRewind(cp.id, rewindMode, overwriteManual)
                              setSelectedCpId(null)
                              setDiffReport(null)
                              setCpOpen(false)
                            } finally {
                              setRewinding(false)
                            }
                          }}
                          className="z-btn z-btn-secondary z-btn-sm font-mono text-[10px]"
                        >
                          <RotateCcw size={10.5} />
                          Rewind
                        </button>
                      </div>
                    </div>

                    {isOpen && (
                      <div className="mt-2 border-t border-[var(--border)]/70 pt-2 font-mono text-[10px]">
                        {loadingDiff ? (
                          <div className="text-[var(--muted)]">Computing checkpoint diff...</div>
                        ) : diffReport ? (
                          <>
                            {diffReport.manual_edit_conflicts.length > 0 && (
                              <Alert variant="warning" className="mb-2 px-2.5 py-1.5 text-[10.5px]">
                                <AlertIcon />
                                <AlertContent>
                                  <AlertDescription className="text-[10.5px] text-[var(--fg)]">
                                    Manual edits kept unless Overwrite is checked:{' '}
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
                              className="rounded border border-[var(--border)] bg-[var(--surface)]"
                              viewportClassName="max-h-40 p-2 pr-3"
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
      )}
    </>
  )
}
