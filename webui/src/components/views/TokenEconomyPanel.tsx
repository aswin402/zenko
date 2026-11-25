import { useCallback, useEffect, useState } from 'react'
import {
  Activity,
  Check,
  Copy,
  Plus,
  RefreshCw,
  RotateCcw,
  Scissors,
  Search,
  Trash2,
} from 'lucide-react'
import { api, formatCompactTokens } from '@/lib/api'
import type {
  CodeMinimalismMode,
  DebtScanReport,
  EconomyConfig,
  MinimalismAuditPayload,
  TokenEconomyReport,
} from '@/lib/types'
import { cn } from '@/lib/utils'

interface EconomyToggleMeta {
  key:
    | 'auto_micro_skills'
    | 'auto_tool_prune'
    | 'p90_loop_guard'
    | 'optional_fast_judge'
    | 'auto_bloat_hint'
  label: string
  hint: string
}

const ECONOMY_TOGGLES: readonly EconomyToggleMeta[] = [
  {
    key: 'auto_micro_skills',
    label: 'Auto Micro-Skills',
    hint: 'Learns recurring workspace fail->fix facts across sessions (zero LLM) and auto-retires on 1 contradiction.',
  },
  {
    key: 'auto_tool_prune',
    label: 'Smart Tool Pruning',
    hint: 'Soft-demotes persistently unused non-core tools to deferred (still 100% callable via invoke_tool) with periodic re-audit.',
  },
  {
    key: 'p90_loop_guard',
    label: 'P90 Loop Guard',
    hint: 'Tracks empirical repo turn baselines (1.25x P90) and redirects subtle arg-varying error oscillations.',
  },
  {
    key: 'auto_bloat_hint',
    label: 'Auto Diff Bloat Guard',
    hint: 'Issues a single 1-shot minimalism check when additive turn diffs or new files cross the active mode threshold.',
  },
  {
    key: 'optional_fast_judge',
    label: 'Fast-Model Loop Judge (Opt-in)',
    hint: 'Uses fast_model for semantic loop diagnosis when P90 breaches; logs exact overhead in the Net ROI Ledger.',
  },
]

const MINIMALISM_LEVELS: readonly {
  id: CodeMinimalismMode
  label: string
  hint: string
}[] = [
  { id: 'off', label: 'Off', hint: 'No ladder constraints' },
  { id: 'lite', label: 'Lite', hint: 'Reuse & stdlib first' },
  { id: 'full', label: 'Full', hint: 'Strict 7-rung ladder' },
  { id: 'ultra', label: 'Ultra', hint: 'Ruthless consolidation' },
]

const STATUS_BADGE_LABEL: Record<string, string> = {
  active: 'ACTIVE',
  candidate: 'CANDIDATE',
  retired: 'RETIRED',
}

/**
 * Native Token Economy Engine Control Panel (Settings & Setup).
 * Strictly inherits global 60/30/10 theme tokens (zero hardcoded colors).
 */
export function TokenEconomyPanel({
  toast,
  syncKey,
  onEconomyConfigChanged,
}: {
  toast: (msg: string, kind?: 'info' | 'success' | 'error') => void
  syncKey?: string
  onEconomyConfigChanged?: (zmodeCustomized?: boolean) => void
}) {
  const [report, setReport] = useState<TokenEconomyReport | null>(null)
  const [loading, setLoading] = useState(false)
  const [busy, setBusy] = useState(false)
  const [newFact, setNewFact] = useState('')
  const [newFixedCmd, setNewFixedCmd] = useState('')
  const [showDetails, setShowDetails] = useState(false)
  const [debtReport, setDebtReport] = useState<DebtScanReport | null>(null)
  const [scanningDebt, setScanningDebt] = useState(false)
  const [auditPayload, setAuditPayload] = useState<MinimalismAuditPayload | null>(null)
  const [runningAudit, setRunningAudit] = useState(false)
  const [copiedSummary, setCopiedSummary] = useState(false)

  const refresh = useCallback(async () => {
    setLoading(true)
    try {
      const data = await api<TokenEconomyReport>('/api/economy')
      setReport(data)
    } catch {
      /* fail open when daemon is starting */
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    refresh()
  }, [refresh, syncKey])

  const updateEconomyCfg = async (patch: Partial<EconomyConfig>) => {
    setBusy(true)
    try {
      const res = await api<{ ok: boolean; economy?: EconomyConfig; zmode_customized?: boolean }>(
        '/api/economy/config',
        {
          method: 'PATCH',
          body: JSON.stringify(patch),
        },
      )
      if (res.economy) {
        setReport((prev) => (prev ? { ...prev, config: res.economy! } : prev))
      }
      onEconomyConfigChanged?.(res.zmode_customized)
      toast('Updated Token Economy settings', 'success')
    } catch (e: any) {
      toast(e?.message || 'Failed to update Token Economy config', 'error')
    } finally {
      setBusy(false)
    }
  }

  const addMicroSkill = async () => {
    if (!newFact.trim()) return
    setBusy(true)
    try {
      await api('/api/economy/micro-skills', {
        method: 'POST',
        body: JSON.stringify({
          fact: newFact.trim(),
          fixed_cmd: newFixedCmd.trim() || null,
        }),
      })
      setNewFact('')
      setNewFixedCmd('')
      toast('Added active repository micro-skill', 'success')
      await refresh()
    } catch (e: any) {
      toast(e?.message || 'Failed to add micro-skill', 'error')
    } finally {
      setBusy(false)
    }
  }

  const retireSkill = async (id: string) => {
    setBusy(true)
    try {
      await api(`/api/economy/micro-skills/${encodeURIComponent(id)}`, { method: 'DELETE' })
      toast('Retired micro-skill', 'info')
      await refresh()
    } catch (e: any) {
      toast(e?.message || 'Failed to retire micro-skill', 'error')
    } finally {
      setBusy(false)
    }
  }

  const restoreTool = async (toolName: string, pin = true) => {
    setBusy(true)
    try {
      await api('/api/economy/tools/restore', {
        method: 'POST',
        body: JSON.stringify({ tool_name: toolName, pin }),
      })
      toast(`Restored ${toolName} (${pin ? 'pinned' : 'unpinned'})`, 'success')
      await refresh()
    } catch (e: any) {
      toast(e?.message || 'Failed to restore tool', 'error')
    } finally {
      setBusy(false)
    }
  }

  const pruneTool = async (toolName: string) => {
    setBusy(true)
    try {
      await api('/api/economy/tools/prune', {
        method: 'POST',
        body: JSON.stringify({ tool_name: toolName }),
      })
      toast(`Soft-pruned ${toolName} to deferred`, 'info')
      await refresh()
    } catch (e: any) {
      toast(e?.message || 'Failed to prune tool', 'error')
    } finally {
      setBusy(false)
    }
  }

  const reauditTools = async () => {
    setBusy(true)
    try {
      const res = await api<{ ok: boolean; restored: number }>('/api/economy/tools/reaudit', {
        method: 'POST',
      })
      toast(`Re-audited tools (${res.restored} restored for fresh evaluation)`, 'info')
      await refresh()
    } catch (e: any) {
      toast(e?.message || 'Failed to re-audit tools', 'error')
    } finally {
      setBusy(false)
    }
  }

  const scanDebt = async () => {
    setScanningDebt(true)
    try {
      const data = await api<DebtScanReport>('/api/economy/minimalism/debt')
      setDebtReport(data)
      setShowDetails(true)
      toast(
        `Scanned ${data.files_scanned} source file(s): ${data.total_found} shortcut debt marker(s)`,
        'info',
      )
      await refresh()
    } catch (e: any) {
      toast(e?.message || 'Failed to scan shortcut debt', 'error')
    } finally {
      setScanningDebt(false)
    }
  }

  const runBloatAudit = async () => {
    setRunningAudit(true)
    try {
      const data = await api<MinimalismAuditPayload>('/api/economy/minimalism/audit')
      setAuditPayload(data)
      setShowDetails(true)
      toast(
        `Audited ${data.repo.files_scanned} files · ${(data.repo.single_impl_abstractions ?? []).length} 1-impl trait/iface · ${(data.repo.comment_bloat ?? []).length} comment-bloat · ${data.repo.dormant_dependencies.length} dormant dep(s)`,
        'info',
      )
      await refresh()
    } catch (e: any) {
      toast(e?.message || 'Failed to run code minimalism audit', 'error')
    } finally {
      setRunningAudit(false)
    }
  }

  const copyMarkdownSummary = async () => {
    if (!report) return
    const s = report.summary
    const m = report.code_metrics
    const churnTotal = (m?.lines_added ?? 0) + (m?.lines_deleted ?? 0)
    const delRatio = churnTotal > 0 ? (((m?.lines_deleted ?? 0) / churnTotal) * 100).toFixed(1) : '0.0'
    const md = [
      `### Zenko Efficiency Report (\`${report.repo_scope}\`)`,
      `- **Net Token ROI:** ${s.net_tokens_saved >= 0 ? '+' : ''}${s.net_tokens_saved} tok (~$${s.estimated_usd_saved.toFixed(4)}) [Read-Less +${s.read_less_tokens_saved} · Output-Dedup +${s.output_dedup_tokens_saved ?? 0} (${s.output_dedup_hits ?? 0} hits) · Tool-Prune +${s.tool_prune_tokens_saved} · Overhead -${s.overhead_tokens} · CCR Recoveries ${s.ccr_recoveries ?? 0}]`,
      `- **Micro-Skills:** ${s.active_micro_skills} active, ${s.candidate_micro_skills} candidate, ${s.retired_micro_skills} retired (${s.micro_skill_injections} injections)`,
      `- **Tool Pruning & Loop Guard:** ${s.pruned_tools_count} soft-deferred · P90=${s.p90_turn_calls} (envelope ${s.envelope_calls}, ${s.loop_guard_interventions} redirects)`,
      `- **Code Minimalism (\`${report.config.minimalism_mode}\`):** +${m?.lines_added ?? 0} / -${m?.lines_deleted ?? 0} LOC (deletion ratio ${delRatio}%, ${m?.new_files_created ?? 0} new files, ${m?.bloat_hints_issued ?? 0} hints, ${m?.debt_markers_last_seen ?? 0} debt markers)`,
    ].join('\n')
    try {
      await navigator.clipboard.writeText(md)
      setCopiedSummary(true)
      toast('Copied Markdown efficiency summary', 'success')
      window.setTimeout(() => setCopiedSummary(false), 1800)
    } catch {
      toast('Clipboard unavailable in this browser context', 'error')
    }
  }

  const cfg = report?.config
  const sum = report?.summary
  const codeMetrics = report?.code_metrics
  const minimalismLevels =
    report?.minimalism_modes && report.minimalism_modes.length > 0
      ? report.minimalism_modes.map((m) => ({
          id: m.id,
          label: m.label,
          hint: m.description,
        }))
      : MINIMALISM_LEVELS
  const enabled = cfg?.enabled ?? true
  const activeMinMode: CodeMinimalismMode = cfg?.minimalism_mode ?? 'lite'

  return (
    <div className="z-surface-card flex flex-col gap-3.5 md:col-span-2">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex items-start gap-2.5">
          <div
            className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border"
            style={
              enabled
                ? {
                    borderColor: 'color-mix(in srgb, var(--accent) 45%, var(--border))',
                    backgroundColor: 'color-mix(in srgb, var(--accent) 12%, var(--surface))',
                    color: 'var(--accent)',
                  }
                : {
                    borderColor: 'var(--border)',
                    backgroundColor: 'var(--surface)',
                    color: 'var(--muted)',
                  }
            }
          >
            <Activity size={15} />
          </div>
          <div>
            <div className="flex flex-wrap items-center gap-2">
              <strong className="text-[14px]">Token Economy Engine</strong>
              <span className="rounded-full border border-[var(--border)] bg-[var(--surface)] px-2 py-0.5 font-mono text-[10.5px] text-[var(--muted)]">
                {report?.repo_scope || 'workspace'}
              </span>
              {sum && (
                <span
                  className="rounded-full border px-2 py-0.5 font-mono text-[10.5px]"
                  style={{
                    color: 'var(--accent)',
                    borderColor: 'color-mix(in srgb, var(--accent) 40%, transparent)',
                    backgroundColor: 'color-mix(in srgb, var(--accent) 10%, transparent)',
                  }}
                >
                  Net {sum.net_tokens_saved >= 0 ? '+' : ''}
                  {formatCompactTokens(Math.abs(sum.net_tokens_saved))} tok (~$
                  {sum.estimated_usd_saved.toFixed(4)})
                </span>
              )}
            </div>
            <p className="mt-0.5 text-[12px] text-[var(--muted)]">
              Cross-session experience controller: zero-LLM micro-skills, soft tool pruning, adaptive
              1.25x P90 loop guard, and honest Net ROI ledger (gross savings minus Dream/Judge
              overhead).
            </p>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {report && (
            <button
              type="button"
              onClick={copyMarkdownSummary}
              className="z-btn z-btn-ghost z-btn-sm font-mono text-[10.5px]"
              title="Copy Markdown ROI & Minimalism Summary"
            >
              {copiedSummary ? <Check size={11.5} /> : <Copy size={11.5} />}
              <span>{copiedSummary ? 'Copied' : 'Copy Summary'}</span>
            </button>
          )}
          <button
            type="button"
            onClick={refresh}
            disabled={loading}
            className="z-btn z-btn-ghost z-btn-sm font-mono text-[10.5px]"
          >
            <RefreshCw size={11.5} className={loading ? 'animate-spin' : ''} />
            <span>Refresh</span>
          </button>
          <button
            type="button"
            role="switch"
            aria-label="Toggle Token Economy Engine"
            aria-checked={enabled}
            disabled={busy || !cfg}
            onClick={() => updateEconomyCfg({ enabled: !enabled })}
            className="relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full border transition-colors disabled:opacity-50"
            style={{
              backgroundColor: enabled ? 'var(--accent)' : 'var(--bg)',
              borderColor: enabled ? 'var(--accent)' : 'var(--border)',
            }}
          >
            <span
              className={cn(
                'inline-block h-4 w-4 rounded-full bg-white shadow-xs transition-transform',
                enabled ? 'translate-x-5.5' : 'translate-x-1',
              )}
            />
          </button>
        </div>
      </div>

      {/* Live Net ROI Telemetry Strip */}
      {sum && (
        <div className="grid grid-cols-2 gap-2 sm:grid-cols-5">
          {[
            [
              'Read-Less + Dedup',
              `${formatCompactTokens(
                sum.read_less_tokens_saved +
                  (sum.output_dedup_tokens_saved ?? 0) +
                  sum.tool_prune_tokens_saved,
              )} tok`,
              `${sum.output_dedup_hits ?? 0} dedup · ${sum.ccr_recoveries ?? 0} CCR · -${formatCompactTokens(sum.overhead_tokens)} oh`,
            ],
            [
              'Micro-Skills',
              `${sum.active_micro_skills} active`,
              `${sum.candidate_micro_skills} candidate · ${sum.retired_micro_skills} retired`,
            ],
            [
              'Soft-Pruned Tools',
              `${sum.pruned_tools_count} deferred`,
              `${report?.prune_candidates.length ?? 0} approaching threshold`,
            ],
            [
              'P90 Loop Baseline',
              sum.envelope_calls > 0 ? `${sum.envelope_calls} calls` : 'Learning',
              sum.baseline_samples > 0
                ? `P90=${sum.p90_turn_calls} (${sum.baseline_samples} runs)`
                : 'Needs 5 completed runs',
            ],
            [
              'Code Minimalism',
              codeMetrics
                ? `+${codeMetrics.lines_added} / -${codeMetrics.lines_deleted}`
                : activeMinMode.toUpperCase(),
              codeMetrics
                ? `${codeMetrics.debt_markers_last_seen} debt · ${codeMetrics.bloat_hints_issued} hints`
                : '7-rung Simplicity Ladder',
            ],
          ].map(([label, val, sub]) => (
            <div
              key={label}
              className="flex flex-col gap-0.5 rounded-xl border border-[var(--border)] bg-[var(--bg)]/55 px-3 py-2"
            >
              <span className="text-[10.5px] text-[var(--muted)]">{label}</span>
              <span className="font-mono text-[13px] font-semibold text-[var(--fg)]">{val}</span>
              <span className="font-mono text-[10px] text-[var(--muted)]">{sub}</span>
            </div>
          ))}
        </div>
      )}

      {/* Native Code Minimalism (7-Rung Simplicity Ladder) Selector */}
      {cfg && (
        <div className="flex flex-col gap-2 rounded-xl border border-[var(--border)] bg-[var(--bg)]/50 px-3.5 py-2.5">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex items-center gap-2">
              <Scissors size={13} style={{ color: activeMinMode !== 'off' ? 'var(--accent)' : 'var(--muted)' }} />
              <span className="text-[12.5px] font-medium text-[var(--fg)]">
                Code Minimalism (7-Rung Simplicity Ladder)
              </span>
            </div>
            <div className="flex flex-wrap items-center gap-1.5">
              <button
                type="button"
                disabled={runningAudit}
                onClick={runBloatAudit}
                className="z-btn z-btn-ghost z-btn-sm font-mono text-[10.5px]"
              >
                <Search size={11} />
                <span>{runningAudit ? 'Auditing…' : 'Run Bloat Audit'}</span>
              </button>
              <button
                type="button"
                disabled={scanningDebt}
                onClick={scanDebt}
                className="z-btn z-btn-ghost z-btn-sm font-mono text-[10.5px]"
              >
                {scanningDebt ? 'Scanning…' : 'Scan Shortcut Debt'}
              </button>
            </div>
          </div>
          <div className="grid grid-cols-2 gap-1.5 sm:grid-cols-4">
            {minimalismLevels.map((lvl) => {
              const selected = activeMinMode === lvl.id
              return (
                <button
                  key={lvl.id}
                  type="button"
                  disabled={busy}
                  onClick={() => updateEconomyCfg({ minimalism_mode: lvl.id })}
                  className="flex cursor-pointer flex-col items-start rounded-lg border px-2.5 py-1.5 text-left transition-colors disabled:opacity-50"
                  style={
                    selected
                      ? {
                          borderColor: 'color-mix(in srgb, var(--accent) 55%, var(--border))',
                          backgroundColor: 'color-mix(in srgb, var(--accent) 12%, var(--surface))',
                        }
                      : {
                          borderColor: 'var(--border)',
                          backgroundColor: 'var(--surface)',
                        }
                  }
                >
                  <span
                    className="font-mono text-[11.5px] font-semibold"
                    style={{ color: selected ? 'var(--accent)' : 'var(--fg)' }}
                  >
                    {lvl.label}
                  </span>
                  <span className="text-[10px] text-[var(--muted)]">{lvl.hint}</span>
                </button>
              )
            })}
          </div>
        </div>
      )}

      {/* Feature Toggles */}
      {cfg && (
        <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
          {ECONOMY_TOGGLES.map((item) => {
            const on = Boolean(cfg[item.key])
            return (
              <div
                key={item.key}
                className="flex items-center justify-between gap-3 rounded-xl border border-[var(--border)] bg-[var(--bg)]/50 px-3.5 py-2.5"
              >
                <div>
                  <div className="text-[12.5px] font-medium text-[var(--fg)]">{item.label}</div>
                  <div className="text-[11px] leading-snug text-[var(--muted)]">{item.hint}</div>
                </div>
                <button
                  type="button"
                  role="switch"
                  aria-label={item.label}
                  aria-checked={on}
                  disabled={busy || !enabled}
                  onClick={() => updateEconomyCfg({ [item.key]: !on })}
                  className="relative inline-flex h-5.5 w-10 shrink-0 cursor-pointer items-center rounded-full border transition-colors disabled:opacity-45"
                  style={{
                    backgroundColor: on ? 'var(--accent)' : 'var(--bg)',
                    borderColor: on ? 'var(--accent)' : 'var(--border)',
                  }}
                >
                  <span
                    className={cn(
                      'inline-block h-3.5 w-3.5 rounded-full bg-white shadow-xs transition-transform',
                      on ? 'translate-x-5' : 'translate-x-1',
                    )}
                  />
                </button>
              </div>
            )
          })}
        </div>
      )}

      {/* Expandable Micro-Skills & Pruned Tools Manager */}
      <div className="border-t border-[var(--border)]/70 pt-2.5">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <button
            type="button"
            onClick={() => setShowDetails((v) => !v)}
            className="z-btn z-btn-ghost z-btn-sm font-mono text-[11px]"
          >
            {showDetails
              ? 'Hide Micro-Skills & Pruned Tools'
              : `Inspect Micro-Skills (${report?.micro_skills.length ?? 0}) & Tool Pruning (${report?.pruned_tools.length ?? 0})`}
          </button>
          {(report?.pruned_tools.length ?? 0) > 0 && (
            <button
              type="button"
              disabled={busy}
              onClick={reauditTools}
              className="z-btn z-btn-secondary z-btn-sm font-mono text-[10.5px]"
            >
              <RotateCcw size={11} /> Re-Audit Pruned Tools
            </button>
          )}
        </div>

        {showDetails && report && (
          <div className="mt-3 grid grid-cols-1 gap-3 md:grid-cols-2">
            {/* Micro-Skills Column */}
            <div className="flex flex-col gap-2 rounded-xl border border-[var(--border)] bg-[var(--bg)]/50 p-3">
              <span className="text-[12px] font-semibold text-[var(--fg)]">
                Repository Micro-Skills ({report.micro_skills.length})
              </span>
              <div className="flex flex-col gap-1.5">
                <input
                  value={newFact}
                  onChange={(e) => setNewFact(e.target.value)}
                  placeholder="Add verified 1-line fact (e.g. Use just verify for Rust tests)"
                  className="w-full rounded-lg border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1.5 text-[11.5px] text-[var(--fg)] placeholder:text-[var(--muted)]/60 focus:border-[var(--accent)] focus:outline-none"
                />
                <div className="flex gap-1.5">
                  <input
                    value={newFixedCmd}
                    onChange={(e) => setNewFixedCmd(e.target.value)}
                    placeholder="Optional working command (e.g. just verify)"
                    className="flex-1 rounded-lg border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1 font-mono text-[11px] text-[var(--fg)] placeholder:text-[var(--muted)]/60 focus:border-[var(--accent)] focus:outline-none"
                  />
                  <button
                    type="button"
                    disabled={busy || !newFact.trim()}
                    onClick={addMicroSkill}
                    className="z-btn z-btn-secondary z-btn-sm"
                  >
                    <Plus size={12} /> Add
                  </button>
                </div>
              </div>

              <div className="flex max-h-48 flex-col gap-1.5 overflow-y-auto">
                {report.micro_skills.length === 0 ? (
                  <span className="font-mono text-[11px] text-[var(--muted)]">
                    No micro-skills learned yet — Zenko extracts recurring fail-&gt;fix facts
                    automatically across sessions.
                  </span>
                ) : (
                  report.micro_skills.map((ms) => (
                    <div
                      key={ms.id}
                      className="flex items-start justify-between gap-2 rounded-lg border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1.5"
                    >
                      <div className="min-w-0">
                        <div className="flex items-center gap-1.5 font-mono text-[10px]">
                          <span
                            style={{
                              color:
                                ms.status === 'active' ? 'var(--accent)' : 'var(--muted)',
                            }}
                          >
                            {STATUS_BADGE_LABEL[ms.status] || ms.status.toUpperCase()}
                          </span>
                          <span className="text-[var(--muted)]">
                            {ms.corroborations}x · hits {ms.need_hits}/{ms.exposures}
                          </span>
                        </div>
                        <p className="mt-0.5 text-[11.5px] leading-snug text-[var(--fg)]">
                          {ms.fact}
                        </p>
                      </div>
                      {ms.status !== 'retired' && (
                        <button
                          type="button"
                          disabled={busy}
                          onClick={() => retireSkill(ms.id)}
                          title="Retire micro-skill"
                          className="z-btn z-btn-ghost z-btn-sm shrink-0 px-1.5 text-[var(--muted)] hover:text-[var(--fg)]"
                        >
                          <Trash2 size={11.5} />
                        </button>
                      )}
                    </div>
                  ))
                )}
              </div>
            </div>

            {/* Tool Utilization & Soft-Pruning Column */}
            <div className="flex flex-col gap-2 rounded-xl border border-[var(--border)] bg-[var(--bg)]/50 p-3">
              <span className="text-[12px] font-semibold text-[var(--fg)]">
                Tool Utilization &amp; Soft Pruning ({report.tool_usage.length})
              </span>
              <div className="flex max-h-60 flex-col gap-1.5 overflow-y-auto">
                {report.tool_usage.length === 0 ? (
                  <span className="font-mono text-[11px] text-[var(--muted)]">
                    No tool telemetry collected yet for this repository.
                  </span>
                ) : (
                  report.tool_usage.slice(0, 20).map((tu) => (
                    <div
                      key={tu.tool_name}
                      className="flex items-center justify-between gap-2 rounded-lg border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1.5"
                    >
                      <div className="min-w-0">
                        <div className="flex items-center gap-1.5 font-mono text-[11px] text-[var(--fg)]">
                          <span className="truncate">{tu.tool_name}</span>
                          {tu.pruned_to_deferred && (
                            <span
                              className="rounded border px-1 py-px text-[9.5px]"
                              style={{
                                color: 'var(--accent)',
                                borderColor: 'color-mix(in srgb, var(--accent) 40%, transparent)',
                              }}
                            >
                              deferred
                            </span>
                          )}
                          {tu.manually_pinned && (
                            <span className="rounded border border-[var(--border)] px-1 py-px text-[9.5px] text-[var(--muted)]">
                              pinned
                            </span>
                          )}
                        </div>
                        <div className="font-mono text-[10px] text-[var(--muted)]">
                          calls={tu.call_count} · idle={tu.prompts_since_used}p · ~
                          {tu.estimated_schema_tokens} tok
                        </div>
                      </div>
                      {tu.pruned_to_deferred ? (
                        <button
                          type="button"
                          disabled={busy}
                          onClick={() => restoreTool(tu.tool_name, true)}
                          className="z-btn z-btn-secondary z-btn-sm shrink-0 font-mono text-[10px]"
                        >
                          Restore
                        </button>
                      ) : (
                        <button
                          type="button"
                          disabled={busy}
                          onClick={() => pruneTool(tu.tool_name)}
                          className="z-btn z-btn-ghost z-btn-sm shrink-0 font-mono text-[10px]"
                        >
                          Defer
                        </button>
                      )}
                    </div>
                  ))
                )}
              </div>
            </div>
            {auditPayload && (
              <div className="flex flex-col gap-2.5 rounded-xl border border-[var(--border)] bg-[var(--bg)]/50 p-3 md:col-span-2">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <span className="text-[12px] font-semibold text-[var(--fg)]">
                    Code Minimalism Audit ({auditPayload.repo.files_scanned} files ·{' '}
                    {auditPayload.repo.total_lines} LOC)
                  </span>
                  <span className="font-mono text-[10.5px] text-[var(--muted)]">
                    Diff: +{auditPayload.diff.lines_added} / -{auditPayload.diff.lines_deleted} LOC
                    ({auditPayload.diff.files_changed.length} files,{' '}
                    {auditPayload.diff.new_files.length} new)
                  </span>
                </div>

                {auditPayload.diff.findings.length > 0 && (
                  <div className="flex flex-col gap-1 rounded-lg border border-[var(--border)] bg-[var(--surface)] px-2.5 py-2 font-mono text-[10.5px]">
                    <span className="font-semibold" style={{ color: 'var(--accent)' }}>
                      Diff Findings
                    </span>
                    {auditPayload.diff.findings.map((f, idx) => (
                      <span key={idx} className="text-[var(--fg)]">
                        • {f}
                      </span>
                    ))}
                  </div>
                )}

                <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                  <div className="flex flex-col gap-1 rounded-lg border border-[var(--border)] bg-[var(--surface)] p-2 font-mono text-[10.5px]">
                    <span className="text-[var(--muted)]">
                      Dormant Dependencies ({auditPayload.repo.dormant_dependencies.length})
                    </span>
                    {auditPayload.repo.dormant_dependencies.length === 0 ? (
                      <span className="text-[var(--muted)]">None detected.</span>
                    ) : (
                      auditPayload.repo.dormant_dependencies.slice(0, 8).map((dep, i) => (
                        <span key={i} className="truncate text-[var(--fg)]">
                          <span style={{ color: 'var(--accent)' }}>{dep.package}</span>{' '}
                          <span className="text-[var(--muted)]">({dep.manifest})</span>
                        </span>
                      ))
                    )}
                  </div>

                  <div className="flex flex-col gap-1 rounded-lg border border-[var(--border)] bg-[var(--surface)] p-2 font-mono text-[10.5px]">
                    <span className="text-[var(--muted)]">
                      Duplicate Helpers ({auditPayload.repo.duplicate_symbols.length}) &amp; Thin
                      Wrappers ({auditPayload.repo.thin_wrappers.length})
                    </span>
                    {auditPayload.repo.duplicate_symbols.length === 0 &&
                    auditPayload.repo.thin_wrappers.length === 0 ? (
                      <span className="text-[var(--muted)]">None detected.</span>
                    ) : (
                      <>
                        {auditPayload.repo.duplicate_symbols.slice(0, 4).map((dup, i) => (
                          <span key={`d-${i}`} className="truncate text-[var(--fg)]">
                            dup <span style={{ color: 'var(--accent)' }}>{dup.symbol}</span> ·{' '}
                            {dup.locations.slice(0, 2).join(', ')}
                          </span>
                        ))}
                        {auditPayload.repo.thin_wrappers.slice(0, 4).map((w, i) => (
                          <span key={`w-${i}`} className="truncate text-[var(--fg)]">
                            wrap <span style={{ color: 'var(--accent)' }}>{w.name}</span> ·{' '}
                            {w.path}:L{w.line} ({w.span_lines}L)
                          </span>
                        ))}
                      </>
                    )}
                  </div>

                  <div className="flex flex-col gap-1 rounded-lg border border-[var(--border)] bg-[var(--surface)] p-2 font-mono text-[10.5px]">
                    <span className="text-[var(--muted)]">
                      Single-Impl Abstractions (
                      {(auditPayload.repo.single_impl_abstractions ?? []).length})
                    </span>
                    {(auditPayload.repo.single_impl_abstractions ?? []).length === 0 ? (
                      <span className="text-[var(--muted)]">None detected.</span>
                    ) : (
                      (auditPayload.repo.single_impl_abstractions ?? [])
                        .slice(0, 6)
                        .map((abs, i) => (
                          <span key={`a-${i}`} className="truncate text-[var(--fg)]">
                            {abs.kind}{' '}
                            <span style={{ color: 'var(--accent)' }}>{abs.name}</span> ·{' '}
                            {abs.path}:L{abs.line} ({abs.impl_count} impl)
                          </span>
                        ))
                    )}
                  </div>

                  <div className="flex flex-col gap-1 rounded-lg border border-[var(--border)] bg-[var(--surface)] p-2 font-mono text-[10.5px]">
                    <span className="text-[var(--muted)]">
                      Comment / Docstring Bloat (
                      {(auditPayload.repo.comment_bloat ?? []).length})
                    </span>
                    {(auditPayload.repo.comment_bloat ?? []).length === 0 ? (
                      <span className="text-[var(--muted)]">None detected.</span>
                    ) : (
                      (auditPayload.repo.comment_bloat ?? []).slice(0, 6).map((cb, i) => (
                        <span key={`c-${i}`} className="truncate text-[var(--fg)]">
                          <span style={{ color: 'var(--accent)' }}>{cb.comment_ratio_pct}%</span> ·{' '}
                          {cb.path}:L{Math.max(1, cb.longest_block_line)} (max {cb.longest_block_lines}L)
                        </span>
                      ))
                    )}
                  </div>
                </div>
              </div>
            )}
            {debtReport && (
              <div className="flex flex-col gap-2 rounded-xl border border-[var(--border)] bg-[var(--bg)]/50 p-3 md:col-span-2">
                <span className="text-[12px] font-semibold text-[var(--fg)]">
                  Tracked Shortcut Debt ({debtReport.total_found} across {debtReport.files_scanned}{' '}
                  files)
                </span>
                <div className="flex max-h-44 flex-col gap-1 overflow-y-auto font-mono text-[10.5px]">
                  {debtReport.items.length === 0 ? (
                    <span className="text-[var(--muted)]">
                      Zero shortcut debt markers found in source comments.
                    </span>
                  ) : (
                    debtReport.items.slice(0, 30).map((d, idx) => (
                      <div
                        key={`${d.path}:${d.line}:${idx}`}
                        className="flex items-baseline gap-2 rounded-md border border-[var(--border)] bg-[var(--surface)] px-2 py-1"
                      >
                        <span style={{ color: 'var(--accent)' }}>{d.marker}</span>
                        <span className="shrink-0 text-[var(--muted)]">
                          {d.path}:L{d.line}
                        </span>
                        <span className="truncate text-[var(--fg)]">{d.text}</span>
                      </div>
                    ))
                  )}
                </div>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  )
}
