import { useEffect, useState } from 'react'
import { History, Plus, Trash2 } from 'lucide-react'
import { api } from '@/lib/api'
import type {
  CronJob,
  GoalLoopRecord,
  GoalLoopRun,
  GoalLoopStatus,
  HubCategoryMeta,
  HubEntry,
  TriggerStatusEntry,
} from '@/lib/types'
import { Tabs, TabsList, TabsTrigger } from '@/components/motion/tabs'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { ZenkoSelect } from '@/components/ui/ZenkoSelect'
import { ScrollPage, splitShellArgs, type SecondaryTabsVariant } from './shared'

// ─── Skills, Cron & Policy View ───────────────────────────────────────────────

// Strict 60/30/10 semantic styling lookup for Goal Loop states (zero competing rainbow hues)
const GOAL_STATUS_STYLE: Record<GoalLoopStatus, { border: string; color: string }> = {
  active: { border: 'var(--accent)', color: 'var(--accent)' },
  succeeded: { border: 'var(--accent)', color: 'var(--accent)' },
  paused: { border: 'var(--border)', color: 'var(--muted)' },
  failed: { border: 'var(--destructive)', color: 'var(--destructive)' },
  stalled: { border: 'var(--destructive)', color: 'var(--destructive)' },
  budget_exhausted: { border: 'var(--destructive)', color: 'var(--destructive)' },
}

export function AutomationView({
  toast,
  hubDir = '',
  hubCategories = [],
  tabsVariant = 'pill',
  smoothScroll = true,
}: {
  toast: (msg: string, kind?: 'info' | 'success' | 'error') => void
  hubDir?: string
  hubCategories?: HubCategoryMeta[]
  tabsVariant?: SecondaryTabsVariant
  smoothScroll?: boolean
}) {
  const [jobs, setJobs] = useState<CronJob[]>([])
  const [goals, setGoals] = useState<GoalLoopRecord[]>([])
  const [triggers, setTriggers] = useState<TriggerStatusEntry[]>([])
  const [policy, setPolicy] = useState<any>(null)
  const [hubEntries, setHubEntries] = useState<HubEntry[]>([])
  const [hubCat, setHubCat] = useState<string>('all')
  const [name, setName] = useState('')
  const [expr, setExpr] = useState('0 9 * * *')
  const [kind, setKind] = useState<'cron' | 'heartbeat' | 'script'>('cron')
  const [prompt, setPrompt] = useState('')
  const [scriptPath, setScriptPath] = useState('')
  const [goalName, setGoalName] = useState('')
  const [goalText, setGoalText] = useState('')
  const [goalVerify, setGoalVerify] = useState('')
  const [goalMaxIter, setGoalMaxIter] = useState('')
  const [goalMaxBudget, setGoalMaxBudget] = useState('')
  const [goalNotify, setGoalNotify] = useState<'webui' | 'telegram' | 'all'>('webui')
  const [expandedGoalId, setExpandedGoalId] = useState<string | null>(null)
  const [goalRunsById, setGoalRunsById] = useState<Record<string, GoalLoopRun[]>>({})
  const [triggerForm, setTriggerForm] = useState<'none' | 'webhook' | 'watcher'>('none')
  const [whId, setWhId] = useState('')
  const [whSource, setWhSource] = useState('')
  const [whPointer, setWhPointer] = useState('')
  const [whValues, setWhValues] = useState('')
  const [whPrompt, setWhPrompt] = useState('')
  const [wtId, setWtId] = useState('')
  const [wtPath, setWtPath] = useState('.')
  const [wtGlob, setWtGlob] = useState('**/*.rs')
  const [wtAction, setWtAction] = useState<'prompt' | 'script' | 'goal_loop'>('prompt')
  const [wtTarget, setWtTarget] = useState('')

  const toggleGoalRuns = async (gid: string) => {
    if (expandedGoalId === gid) {
      setExpandedGoalId(null)
      return
    }
    setExpandedGoalId(gid)
    try {
      const detail = await api<{ goal: GoalLoopRecord; runs: GoalLoopRun[] }>(
        `/api/goals/${encodeURIComponent(gid)}`,
      )
      setGoalRunsById((prev) => ({ ...prev, [gid]: detail.runs || [] }))
    } catch (e: any) {
      toast(e.message, 'error')
    }
  }

  const load = async () => {
    const res = await api<{
      jobs: CronJob[]
      goals?: GoalLoopRecord[]
      triggers?: TriggerStatusEntry[]
      policy: any
    }>('/api/automation')
    setJobs(res.jobs)
    setGoals(res.goals || [])
    setTriggers(res.triggers || [])
    setPolicy(res.policy)
    api<{ entries: HubEntry[] }>(`/api/hub${hubCat !== 'all' ? `?category=${encodeURIComponent(hubCat)}` : ''}`)
      .then((h) => setHubEntries(h.entries || []))
      .catch(() => {})
  }
  useEffect(() => { load() }, [hubCat])

  return (
    <ScrollPage smoothScroll={smoothScroll}>
      <div className="mx-auto flex w-full max-w-[860px] flex-col gap-6">
        <div>
          <h1 className="text-[22px] font-semibold tracking-[-0.03em]" style={{ fontFamily: 'var(--font-display)' }}>
            Cron, Zero-Token Scripts & Zenko Hub
          </h1>
          <p className="mt-1 text-[13px] text-[var(--muted)]">
            Zero-CPU `sleep_until` scheduler with LLM prompts or zero-token scripts from{' '}
            <code className="font-mono">{hubDir || '~/zenko_hub'}/scripts/</code>.
          </p>
        </div>

        <div className="z-surface-card flex flex-col gap-3">
          <strong className="text-[14px]">Schedule 24/7 Task</strong>
          <div className="grid gap-2.5 sm:grid-cols-3">
            <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Job name" className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 text-[13px]" />
            <input value={expr} onChange={(e) => setExpr(e.target.value)} placeholder="0 9 * * *" className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 font-mono text-[13px]" />
            <ZenkoSelect
              ariaLabel="Scheduled Task Kind"
              value={kind}
              onChange={(v) => setKind(v as any)}
              options={[
                { value: 'cron', label: 'Cron Job (LLM)' },
                { value: 'heartbeat', label: 'Autonomous Heartbeat' },
                { value: 'script', label: 'Script Only (0 tokens)' },
              ]}
            />
          </div>
          {kind === 'script' ? (
            <input
              value={scriptPath}
              onChange={(e) => setScriptPath(e.target.value)}
              placeholder="Script file inside ~/zenko_hub/scripts/ (e.g. backup.sh or health.py)"
              className="w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 font-mono text-[13px]"
            />
          ) : (
            <textarea
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
              rows={2}
              placeholder="Prompt Zenko will execute autonomously..."
              className="w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] p-3 text-[13px]"
            />
          )}
          <button
            type="button"
            className="z-btn z-btn-primary z-btn-sm self-start"
            onClick={async () => {
              try {
                await api('/api/cron', {
                  method: 'POST',
                  body: JSON.stringify({
                    name: name || 'Scheduled Task',
                    cron_expr: expr,
                    kind,
                    prompt: kind === 'script' ? '' : prompt,
                    script_path: kind === 'script' ? scriptPath.trim() : null,
                  }),
                })
                setName('')
                setPrompt('')
                setScriptPath('')
                toast('Job scheduled', 'success')
                load()
              } catch (e: any) {
                toast(e.message, 'error')
              }
            }}
          >
            <Plus size={13} /> Schedule Job
          </button>
        </div>

        <div className="z-surface-card flex flex-col gap-2.5">
          <strong className="text-[14px]">Scheduled Jobs ({jobs.length})</strong>
          {jobs.length === 0 ? (
            <p className="text-[13px] text-[var(--muted)]">No scheduled jobs yet.</p>
          ) : (
            jobs.map((j) => (
              <div
                key={j.id}
                className="flex flex-wrap items-center justify-between gap-2 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3.5 py-2.5 text-[13px]"
              >
                <div className="min-w-0 flex-1">
                  <span className="font-medium">{j.name}</span>
                  <span className="ml-2 rounded border border-[var(--border)] px-1.5 py-0.5 font-mono text-[10.5px] uppercase text-[var(--muted)]">
                    {j.kind}
                  </span>
                  <span className="ml-1.5 rounded border border-[var(--border)] px-1.5 py-0.5 font-mono text-[11px] text-[var(--accent)]">
                    {j.cron_expr}
                  </span>
                  {!j.enabled && (
                    <span className="ml-1.5 font-mono text-[10.5px] text-[var(--muted)]">paused</span>
                  )}
                  <div className="mt-0.5 truncate font-mono text-[11.5px] text-[var(--muted)]">
                    {j.script_path ? `hub:scripts/${j.script_path}` : j.prompt}
                  </div>
                </div>
                <div className="flex shrink-0 items-center gap-1.5">
                  <button
                    type="button"
                    onClick={async () => {
                      try {
                        await api(`/api/cron/${encodeURIComponent(j.id)}/run`, { method: 'POST' })
                        toast(`Dispatched ${j.name}`, 'success')
                        load()
                      } catch (e: any) {
                        toast(e.message, 'error')
                      }
                    }}
                    className="z-btn z-btn-secondary z-btn-sm"
                  >
                    Run
                  </button>
                  <button
                    type="button"
                    onClick={async () => {
                      await api(`/api/cron/${encodeURIComponent(j.id)}`, {
                        method: 'PATCH',
                        body: JSON.stringify({ enabled: !j.enabled }),
                      })
                      load()
                    }}
                    className="z-btn z-btn-ghost z-btn-sm"
                  >
                    {j.enabled ? 'Pause' : 'Resume'}
                  </button>
                  <button
                    type="button"
                    onClick={async () => {
                      await api(`/api/cron/${encodeURIComponent(j.id)}`, { method: 'DELETE' })
                      load()
                    }}
                    className="z-btn z-btn-danger z-btn-sm"
                  >
                    Delete
                  </button>
                </div>
              </div>
            ))
          )}
        </div>

        <div className="z-surface-card flex flex-col gap-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <strong className="text-[14px]">Zenko Hub ({hubEntries.length})</strong>
              <span className="ml-2 font-mono text-[11px] text-[var(--muted)]">
                {hubDir || '~/zenko_hub'}
              </span>
            </div>
            <Tabs
              variant={tabsVariant}
              value={hubCat}
              onValueChange={setHubCat}
              className="min-w-0 max-w-full"
            >
              <TabsList>
                <TabsTrigger value="all" className="font-mono text-[11px]">
                  All
                </TabsTrigger>
                {hubCategories.map((c) => (
                  <TabsTrigger key={c.id} value={c.id} className="font-mono text-[11px]">
                    {c.label}
                  </TabsTrigger>
                ))}
              </TabsList>
            </Tabs>
          </div>
          {hubEntries.length === 0 ? (
            <p className="font-mono text-[11.5px] text-[var(--muted)]">
              No creations in this category yet — standalone projects, scripts, media, and exports land here automatically.
            </p>
          ) : (
            <CompactScrollArea autoHeight viewportClassName="max-h-48 pr-2.5">
              <div className="flex flex-col gap-1">
                {hubEntries.slice(0, 30).map((e, i) => (
                  <div
                    key={`${e.category}-${e.rel_path}-${i}`}
                    className="flex items-center justify-between rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-1.5 font-mono text-[11.5px]"
                  >
                    <div className="min-w-0 truncate">
                      <span className="mr-2 uppercase text-[var(--muted)]">{e.category}</span>
                      <span className="text-[var(--fg)]">{e.rel_path}</span>
                    </div>
                    <span className="shrink-0 text-[10.5px] text-[var(--muted)]">
                      {e.is_dir ? 'dir' : `${e.size_bytes}B`}
                    </span>
                  </div>
                ))}
              </div>
            </CompactScrollArea>
          )}
        </div>

        {/* Phase 8: Bounded 24/7 Autonomous Verify-Until-Green Goal Loops */}
        <div className="z-surface-card flex flex-col gap-3">
          <div>
            <strong className="text-[14px]">24/7 Autonomous Goal Loops ({goals.length})</strong>
            <p className="text-[12px] text-[var(--muted)]">
              Iterate autonomously with Shadow-Git checkpoints until deterministic `verify_cmd` exits 0, or stop cleanly on budget/stall caps.
            </p>
          </div>
          <div className="grid gap-2 sm:grid-cols-4">
            <input
              value={goalName}
              onChange={(e) => setGoalName(e.target.value)}
              placeholder="Goal title (e.g. Fix Clippy Warnings)"
              className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 text-[12.5px]"
            />
            <input
              value={goalVerify}
              onChange={(e) => setGoalVerify(e.target.value)}
              placeholder="verify argv (e.g. cargo test --workspace)"
              className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 font-mono text-[12px]"
            />
            <div className="grid grid-cols-2 gap-2">
              <input
                value={goalMaxIter}
                onChange={(e) => setGoalMaxIter(e.target.value)}
                placeholder="Max iters"
                title="Max iterations"
                className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-2.5 py-2 font-mono text-[12px]"
              />
              <input
                value={goalMaxBudget}
                onChange={(e) => setGoalMaxBudget(e.target.value)}
                placeholder="Max $"
                title="Max budget USD"
                className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-2.5 py-2 font-mono text-[12px]"
              />
            </div>
            <ZenkoSelect
              mono
              ariaLabel="Completion notification channel"
              title="Completion notification channel"
              value={goalNotify}
              onChange={(v) => setGoalNotify(v as 'webui' | 'telegram' | 'all')}
              options={[
                { value: 'webui', label: 'Notify: WebUI' },
                { value: 'telegram', label: 'Notify: Telegram' },
                { value: 'all', label: 'Notify: All' },
              ]}
            />
          </div>
          <textarea
            value={goalText}
            onChange={(e) => setGoalText(e.target.value)}
            rows={2}
            placeholder="Objective instructions executed each iteration..."
            className="w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] p-2.5 text-[12.5px]"
          />
          <button
            type="button"
            className="z-btn z-btn-primary z-btn-sm self-start"
            onClick={async () => {
              const argv = splitShellArgs(goalVerify)
              if (!goalName.trim() || !goalText.trim() || argv.length === 0) {
                toast('Provide goal title, objective text, and verify_cmd', 'error')
                return
              }
              try {
                const parsedIter = goalMaxIter.trim() ? Number(goalMaxIter) : undefined
                const parsedBudget = goalMaxBudget.trim() ? Number(goalMaxBudget) : undefined
                await api('/api/goals', {
                  method: 'POST',
                  body: JSON.stringify({
                    name: goalName.trim(),
                    goal_text: goalText.trim(),
                    verify_cmd: argv,
                    notify_channel: goalNotify,
                    max_iterations:
                      parsedIter !== undefined && Number.isFinite(parsedIter)
                        ? Math.max(1, Math.floor(parsedIter))
                        : undefined,
                    max_budget_usd:
                      parsedBudget !== undefined && Number.isFinite(parsedBudget)
                        ? Math.max(0, parsedBudget)
                        : undefined,
                  }),
                })
                setGoalName('')
                setGoalText('')
                setGoalVerify('')
                setGoalMaxIter('')
                setGoalMaxBudget('')
                toast('Goal loop created', 'success')
                load()
              } catch (e: any) {
                toast(e.message, 'error')
              }
            }}
          >
            <Plus size={13} /> Start Goal Loop
          </button>

          {goals.length > 0 && (
            <div className="flex flex-col gap-2">
              {goals.map((g) => {
                const badgeStyle = GOAL_STATUS_STYLE[g.status] ?? {
                  border: 'var(--border)',
                  color: 'var(--muted)',
                }
                const isExpanded = expandedGoalId === g.id
                const runs = goalRunsById[g.id] ?? []
                return (
                <div
                  key={g.id}
                  className="flex flex-col gap-2 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3.5 py-2.5 text-[12.5px]"
                >
                  <div className="flex flex-wrap items-center justify-between gap-2">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <span className="font-medium">{g.name}</span>
                      <span
                        className="rounded border px-1.5 py-0.5 font-mono text-[10px] uppercase"
                        style={{
                          borderColor: badgeStyle.border,
                          color: badgeStyle.color,
                        }}
                      >
                        {g.status}
                      </span>
                      <span className="font-mono text-[10.5px] text-[var(--muted)]">
                        iter {g.iterations_done}/{g.max_iterations} · ${g.cost_spent_usd.toFixed(4)}/${g.max_budget_usd.toFixed(2)}
                      </span>
                      {g.last_verify_exit != null && (
                        <span className="font-mono text-[10.5px] text-[var(--muted)]">
                          exit={g.last_verify_exit}
                        </span>
                      )}
                    </div>
                    <div className="mt-0.5 truncate font-mono text-[11px] text-[var(--muted)]">
                      verify: {g.verify_cmd.join(' ')}
                    </div>
                  </div>
                  <div className="flex shrink-0 items-center gap-1.5">
                    <button
                      type="button"
                      className={isExpanded ? 'z-btn z-btn-vermilion-soft z-btn-sm' : 'z-btn z-btn-ghost z-btn-sm'}
                      onClick={() => toggleGoalRuns(g.id)}
                    >
                      <History size={12} /> Runs
                    </button>
                    <button
                      type="button"
                      className="z-btn z-btn-secondary z-btn-sm"
                      onClick={async () => {
                        try {
                          await api(`/api/goals/${encodeURIComponent(g.id)}/run`, { method: 'POST' })
                          toast(`Ran iteration for ${g.name}`, 'success')
                          load()
                          if (expandedGoalId === g.id) {
                            const detail = await api<{ goal: GoalLoopRecord; runs: GoalLoopRun[] }>(
                              `/api/goals/${encodeURIComponent(g.id)}`,
                            )
                            setGoalRunsById((prev) => ({ ...prev, [g.id]: detail.runs || [] }))
                          }
                        } catch (e: any) {
                          toast(e.message, 'error')
                        }
                      }}
                    >
                      Run Iter
                    </button>
                    <button
                      type="button"
                      className="z-btn z-btn-ghost z-btn-sm"
                      onClick={async () => {
                        const nextStatus = g.status === 'active' ? 'paused' : 'active'
                        await api(`/api/goals/${encodeURIComponent(g.id)}`, {
                          method: 'PATCH',
                          body: JSON.stringify({
                            status: nextStatus,
                            reset_stalls: nextStatus === 'active',
                          }),
                        })
                        load()
                      }}
                    >
                      {g.status === 'active' ? 'Pause' : 'Resume'}
                    </button>
                    <button
                      type="button"
                      className="z-btn z-btn-danger z-btn-sm"
                      onClick={async () => {
                        await api(`/api/goals/${encodeURIComponent(g.id)}`, { method: 'DELETE' })
                        if (expandedGoalId === g.id) setExpandedGoalId(null)
                        load()
                      }}
                    >
                      Delete
                    </button>
                  </div>
                  </div>
                  {isExpanded && (
                    <div className="flex flex-col gap-1.5 border-t border-[var(--border)] pt-2">
                      {runs.length === 0 ? (
                        <span className="font-mono text-[11px] text-[var(--muted)]">
                          No iterations recorded yet.
                        </span>
                      ) : (
                        runs.map((r) => (
                          <div
                            key={r.id}
                            className="rounded border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1.5 font-mono text-[11px]"
                          >
                            <div className="flex flex-wrap items-center justify-between gap-2">
                              <span>
                                <strong>#{r.iteration}</strong>{' '}
                                <span
                                  style={{
                                    color:
                                      r.verify_exit_code === 0
                                        ? 'var(--accent)'
                                        : 'var(--destructive)',
                                  }}
                                >
                                  {r.outcome}
                                </span>{' '}
                                · exit={r.verify_exit_code} · reward={r.reward.toFixed(1)}
                              </span>
                              <span className="text-[var(--muted)]">
                                {r.duration_ms}ms · {r.tokens_used} tok · ${r.cost_usd.toFixed(4)}
                                {r.checkpoint_id ? ` · cp:${r.checkpoint_id.slice(0, 10)}` : ''}
                              </span>
                            </div>
                            {r.verify_output_excerpt && (
                              <CompactScrollArea autoHeight className="mt-1" viewportClassName="max-h-24 pr-2.5">
                                <pre className="whitespace-pre-wrap text-[10.5px] text-[var(--muted)]">
                                  {r.verify_output_excerpt}
                                </pre>
                              </CompactScrollArea>
                            )}
                          </div>
                        ))
                      )}
                    </div>
                  )}
                </div>
                )
              })}
            </div>
          )}
        </div>

        {/* Phase 8: Declarative Event Triggers (Webhook Routing Rules & Workspace Watchers) */}
        <div className="z-surface-card flex flex-col gap-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <strong className="text-[14px]">Event Triggers & Watchers ({triggers.length})</strong>
            <div className="flex items-center gap-1.5">
              <button
                type="button"
                onClick={() => setTriggerForm((f) => (f === 'webhook' ? 'none' : 'webhook'))}
                className={triggerForm === 'webhook' ? 'z-btn z-btn-vermilion-soft z-btn-sm' : 'z-btn z-btn-ghost z-btn-sm'}
              >
                <Plus size={12} /> Webhook Rule
              </button>
              <button
                type="button"
                onClick={() => setTriggerForm((f) => (f === 'watcher' ? 'none' : 'watcher'))}
                className={triggerForm === 'watcher' ? 'z-btn z-btn-vermilion-soft z-btn-sm' : 'z-btn z-btn-ghost z-btn-sm'}
              >
                <Plus size={12} /> Watcher
              </button>
            </div>
          </div>

          {triggerForm === 'webhook' && (
            <div className="flex flex-col gap-2 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] p-3">
              <div className="grid gap-2 sm:grid-cols-3">
                <input
                  value={whId}
                  onChange={(e) => setWhId(e.target.value)}
                  placeholder="Rule ID (e.g. gh-ci-fail)"
                  className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1.5 font-mono text-[12px]"
                />
                <input
                  value={whSource}
                  onChange={(e) => setWhSource(e.target.value)}
                  placeholder="Source (e.g. github or *)"
                  className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1.5 font-mono text-[12px]"
                />
                <input
                  value={whPointer}
                  onChange={(e) => setWhPointer(e.target.value)}
                  placeholder="JSON Pointer (e.g. /action)"
                  className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1.5 font-mono text-[12px]"
                />
              </div>
              <div className="grid gap-2 sm:grid-cols-2">
                <input
                  value={whValues}
                  onChange={(e) => setWhValues(e.target.value)}
                  placeholder="Match values comma-separated (optional)"
                  className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1.5 font-mono text-[12px]"
                />
                <input
                  value={whPrompt}
                  onChange={(e) => setWhPrompt(e.target.value)}
                  placeholder="Prompt template ({source}, {rule_id}, {payload})"
                  className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1.5 text-[12px]"
                />
              </div>
              <button
                type="button"
                className="z-btn z-btn-primary z-btn-sm self-start"
                onClick={async () => {
                  if (!whId.trim() || !whSource.trim()) {
                    toast('Provide webhook rule ID and source', 'error')
                    return
                  }
                  try {
                    const match_values = whValues
                      .split(',')
                      .map((v) => v.trim())
                      .filter(Boolean)
                    await api('/api/triggers/webhooks', {
                      method: 'POST',
                      body: JSON.stringify({
                        id: whId.trim(),
                        enabled: true,
                        source: whSource.trim(),
                        match_pointer: whPointer.trim() || null,
                        match_values,
                        prompt_template: whPrompt.trim(),
                      }),
                    })
                    setWhId('')
                    setWhSource('')
                    setWhPointer('')
                    setWhValues('')
                    setWhPrompt('')
                    setTriggerForm('none')
                    toast('Webhook rule saved', 'success')
                    load()
                  } catch (e: any) {
                    toast(e.message, 'error')
                  }
                }}
              >
                Save Webhook Rule
              </button>
            </div>
          )}

          {triggerForm === 'watcher' && (
            <div className="flex flex-col gap-2 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] p-3">
              <div className="grid gap-2 sm:grid-cols-4">
                <input
                  value={wtId}
                  onChange={(e) => setWtId(e.target.value)}
                  placeholder="Watcher ID (e.g. rs-tests)"
                  className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1.5 font-mono text-[12px]"
                />
                <input
                  value={wtPath}
                  onChange={(e) => setWtPath(e.target.value)}
                  placeholder="Path (default .)"
                  className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1.5 font-mono text-[12px]"
                />
                <input
                  value={wtGlob}
                  onChange={(e) => setWtGlob(e.target.value)}
                  placeholder="Glob (e.g. **/*.rs)"
                  className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1.5 font-mono text-[12px]"
                />
                <ZenkoSelect
                  mono
                  ariaLabel="Watcher Action"
                  value={wtAction}
                  onChange={(v) => setWtAction(v as 'prompt' | 'script' | 'goal_loop')}
                  className="bg-[var(--surface)] py-1.5"
                  options={[
                    { value: 'prompt', label: 'Action: Prompt' },
                    { value: 'script', label: 'Action: Script (0 tok)' },
                    { value: 'goal_loop', label: 'Action: Wake Goal' },
                  ]}
                />
              </div>
              <input
                value={wtTarget}
                onChange={(e) => setWtTarget(e.target.value)}
                placeholder={
                  wtAction === 'script'
                    ? 'Script file inside ~/zenko_hub/scripts/ (e.g. test.sh)'
                    : wtAction === 'goal_loop'
                      ? 'Goal Loop ID to wake (e.g. goal_...)'
                      : 'Prompt template ({files}, {watcher_id})'
                }
                className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1.5 text-[12px]"
              />
              <button
                type="button"
                className="z-btn z-btn-primary z-btn-sm self-start"
                onClick={async () => {
                  if (!wtId.trim()) {
                    toast('Provide watcher ID', 'error')
                    return
                  }
                  try {
                    await api('/api/triggers/watchers', {
                      method: 'POST',
                      body: JSON.stringify({
                        id: wtId.trim(),
                        enabled: true,
                        path: wtPath.trim() || '.',
                        glob: wtGlob.trim() || '**/*',
                        action: wtAction,
                        prompt: wtAction === 'prompt' ? wtTarget.trim() : '',
                        script_path: wtAction === 'script' ? wtTarget.trim() || null : null,
                        goal_loop_id: wtAction === 'goal_loop' ? wtTarget.trim() || null : null,
                      }),
                    })
                    setWtId('')
                    setWtTarget('')
                    setTriggerForm('none')
                    toast('Workspace watcher saved', 'success')
                    load()
                  } catch (e: any) {
                    toast(e.message, 'error')
                  }
                }}
              >
                Save Watcher
              </button>
            </div>
          )}

          {triggers.length === 0 ? (
            <p className="font-mono text-[11.5px] text-[var(--muted)]">
              No `[[webhooks.rules]]` or `[[watchers]]` configured (0% CPU idle).
            </p>
          ) : (
            <div className="flex flex-col gap-1.5">
              {triggers.map((t) => (
                <div
                  key={`${t.kind}-${t.id}`}
                  className="flex flex-wrap items-center justify-between gap-2 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 font-mono text-[11.5px]"
                >
                  <div className="min-w-0 flex-1 truncate">
                    <span className="font-semibold text-[var(--fg)]">{t.id}</span>
                    <span className="ml-2 rounded border border-[var(--border)] px-1.5 py-0.5 text-[10px] uppercase text-[var(--muted)]">
                      {t.kind}
                    </span>
                    <span className="ml-2 text-[var(--muted)]">{t.detail}</span>
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    <span className="text-[10.5px] text-[var(--muted)]">
                      {t.fires_last_hour}/{t.max_per_hour}/hr · {t.target}
                    </span>
                    <button
                      type="button"
                      title={`Delete ${t.kind}`}
                      className="z-btn z-btn-danger z-btn-sm"
                      onClick={async () => {
                        const endpoint =
                          t.kind === 'webhook_rule'
                            ? `/api/triggers/webhooks/${encodeURIComponent(t.id)}`
                            : `/api/triggers/watchers/${encodeURIComponent(t.id)}`
                        await api(endpoint, { method: 'DELETE' })
                        toast(`Removed ${t.id}`, 'info')
                        load()
                      }}
                    >
                      <Trash2 size={12} />
                    </button>
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {policy && (
          <div className="z-surface-card flex flex-col gap-2 text-[12.5px]">
            <strong className="text-[14px]">Active ExecPolicy ({policy.mode})</strong>
            <div className="text-[var(--muted)]">Allowlisted read-only prefixes:</div>
            <div className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] p-2.5 font-mono text-[11.5px]">
              {(policy.allowed_shell_prefixes || []).join(', ')}
            </div>
          </div>
        )}
      </div>
    </ScrollPage>
  )
}

