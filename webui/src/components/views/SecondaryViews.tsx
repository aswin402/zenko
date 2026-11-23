import { type ReactNode, useEffect, useRef, useState } from 'react'
import { AlertTriangle, BrainIcon, Check, Download, History, Pencil, Plus, RefreshCw, RotateCcw, Search, Trash2, X } from 'lucide-react'
import { api, formatCompactTokens } from '@/lib/api'
import type {
  BackgroundJob,
  CronJob,
  DaemonTelemetry,
  DreamRunRecord,
  DreamStatus,
  GatewayPeer,
  GoalLoopRecord,
  GoalLoopRun,
  GoalLoopStatus,
  HubCategoryMeta,
  HubEntry,
  JobReadChunk,
  LogCategoryMeta,
  LogEntry,
  LspServerDescriptor,
  LspServerRuntimeStatus,
  McpServerStatus,
  MemoryDocSnapshot,
  MemoryEntry,
  MemoryKindMeta,
  ModelInfo,
  PairingCode,
  ProviderDescriptor,
  ScoredMemoryHit,
  SessionSearchHit,
  SkillVersionSnapshot,
  SubagentRecord,
  SubagentRoleDescriptor,
  TaskKindMeta,
  TriggerStatusEntry,
  WebSearchProviderDescriptor,
  WorktreeDiffReport,
  WorkspaceSnapshot,
} from '@/lib/types'
import { ZenkoFoxLogo } from '@/components/beui/ZenkoFoxLogo'
import { Tabs, TabsList, TabsTrigger } from '@/components/motion/tabs'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { ZenkoSelect } from '@/components/ui/ZenkoSelect'

type SecondaryTabsVariant = 'pill' | 'segment' | 'underline'

function ScrollPage({
  smoothScroll = true,
  children,
}: {
  smoothScroll?: boolean
  children: ReactNode
}) {
  return (
    <CompactScrollArea
      smooth={smoothScroll}
      className="min-h-0 flex-1"
      viewportClassName="px-6 py-8 pr-20 sm:px-10"
    >
      {children}
    </CompactScrollArea>
  )
}

// ─── Setup & Settings View ────────────────────────────────────────────────────

/// Deterministic quote-aware argument splitter (zero regex, preserves quoted paths/headers).
function splitShellArgs(raw: string): string[] {
  const out: string[] = []
  let cur = ''
  let quote: '"' | "'" | null = null
  let escaped = false
  for (let i = 0; i < raw.length; i++) {
    const ch = raw[i]
    if (escaped) {
      cur += ch
      escaped = false
      continue
    }
    if (ch === '\\' && quote !== "'") {
      escaped = true
      continue
    }
    if (quote) {
      if (ch === quote) quote = null
      else cur += ch
      continue
    }
    if (ch === '"' || ch === "'") {
      quote = ch
    } else if (ch === ' ' || ch === '\t' || ch === '\n' || ch === '\r') {
      if (cur.length > 0) {
        out.push(cur)
        cur = ''
      }
    } else {
      cur += ch
    }
  }
  if (cur.length > 0) out.push(cur)
  return out
}

export function SetupView({
  config,
  providers,
  configuredProviders,
  hasTelegram,
  webSearchProviders = [],
  hasBraveSearchKey = false,
  hasTavilySearchKey = false,
  mcpServers = [],
  lspRegistry = [],
  lspActive = [],
  onMcpUpdated,
  onLspUpdated,
  onSaved,
  onRefreshBootstrap,
  tabsVariant = 'pill',
  smoothScroll = true,
  toast,
}: {
  config: any
  providers: ProviderDescriptor[]
  configuredProviders: Record<string, boolean>
  hasTelegram: boolean
  webSearchProviders?: WebSearchProviderDescriptor[]
  hasBraveSearchKey?: boolean
  hasTavilySearchKey?: boolean
  mcpServers?: McpServerStatus[]
  lspRegistry?: LspServerDescriptor[]
  lspActive?: LspServerRuntimeStatus[]
  onMcpUpdated?: (servers: McpServerStatus[]) => void
  onLspUpdated?: (active: LspServerRuntimeStatus[]) => void
  onSaved: () => void
  onRefreshBootstrap?: () => void
  tabsVariant?: SecondaryTabsVariant
  smoothScroll?: boolean
  toast: (msg: string, kind?: 'info' | 'success' | 'error') => void
}) {
  void tabsVariant
  const initialKind = config?.provider?.kind || providers[0]?.id || 'openrouter'
  const [providerKind, setProviderKind] = useState(initialKind)
  const activeDesc =
    providers.find((p) => p.id === providerKind) || providers[0] || null

  const [apiKey, setApiKey] = useState('')
  const [baseUrl, setBaseUrl] = useState(
    config?.provider?.base_url || activeDesc?.default_base_url || '',
  )
  const [leadModel, setLeadModel] = useState(config?.provider?.lead_model || '')
  const [fastModel, setFastModel] = useState(config?.provider?.fast_model || '')
  const [fallbacks, setFallbacks] = useState((config?.provider?.fallbacks || []).join(', '))
  const [discoveredModels, setDiscoveredModels] = useState<ModelInfo[]>([])
  const [loadingModels, setLoadingModels] = useState(false)
  const fetchSeq = useRef(0)
  const [tgToken, setTgToken] = useState('')
  const [tgEnabled, setTgEnabled] = useState(Boolean(config?.telegram?.enabled))
  const [workspaceDir, setWorkspaceDir] = useState(config?.general?.workspace_dir || '.')
  const [budget, setBudget] = useState(String(config?.general?.daily_spend_limit_usd ?? 5))
  const [policyMode, setPolicyMode] = useState(config?.policy?.mode || 'autonomous')
  const [providerStatus, setProviderStatus] = useState('')
  const [tgStatus, setTgStatus] = useState('')
  const [saving, setSaving] = useState(false)

  // Phase 7: Pluggable Web Search + External MCP Client Servers + Pooled LSP
  const [searchEnabled, setSearchEnabled] = useState(config?.web_search?.enabled ?? true)
  const [searchProvider, setSearchProvider] = useState(config?.web_search?.provider || 'auto')
  const [searxngUrl, setSearxngUrl] = useState(config?.web_search?.searxng_url || '')
  const [allowPrivateSearxng, setAllowPrivateSearxng] = useState(
    Boolean(config?.web_search?.allow_private_searxng),
  )
  const [braveKey, setBraveKey] = useState('')
  const [tavilyKey, setTavilyKey] = useState('')
  const [savingSearch, setSavingSearch] = useState(false)

  const [mcpAddOpen, setMcpAddOpen] = useState(false)
  const [mcpName, setMcpName] = useState('')
  const [mcpTransport, setMcpTransport] = useState<'stdio' | 'http'>('stdio')
  const [mcpCommand, setMcpCommand] = useState('')
  const [mcpArgs, setMcpArgs] = useState('')
  const [mcpUrl, setMcpUrl] = useState('')
  const [mcpAuthSecret, setMcpAuthSecret] = useState('')
  const [mcpRisk, setMcpRisk] = useState('')
  const [mcpParallel, setMcpParallel] = useState(false)
  const [mcpAllowPrivate, setMcpAllowPrivate] = useState(false)
  const [mcpBusy, setMcpBusy] = useState<string | null>(null)
  const [expandedMcp, setExpandedMcp] = useState<string | null>(null)

  useEffect(() => {
    if (!config?.web_search) return
    setSearchEnabled(config.web_search.enabled ?? true)
    setSearchProvider(config.web_search.provider || 'auto')
    setSearxngUrl(config.web_search.searxng_url || '')
    setAllowPrivateSearxng(Boolean(config.web_search.allow_private_searxng))
  }, [config?.web_search])

  const hasSelectedKey = Boolean(configuredProviders[providerKind])
  const needsKey = activeDesc ? activeDesc.requires_api_key : true

  const loadLiveModels = async (kind: string, url: string, refresh = false) => {
    const seq = ++fetchSeq.current
    setLoadingModels(true)
    try {
      const qs = new URLSearchParams({ provider: kind })
      if (url.trim()) qs.set('base_url', url.trim())
      if (refresh) qs.set('refresh', 'true')
      const list = await api<ModelInfo[]>(`/api/models?${qs.toString()}`)
      if (seq !== fetchSeq.current) return
      setDiscoveredModels(list)
      setLeadModel((prev: string) => prev.trim() || list[0]?.id || '')
      setFastModel((prev: string) => prev.trim() || list[0]?.id || '')
    } catch {
      if (seq === fetchSeq.current) setDiscoveredModels([])
    } finally {
      if (seq === fetchSeq.current) setLoadingModels(false)
    }
  }

  useEffect(() => {
    loadLiveModels(providerKind, baseUrl)
  }, [providerKind])

  const selectProvider = (nextKind: string) => {
    setProviderKind(nextKind)
    setApiKey('')
    setProviderStatus('')
    const desc = providers.find((p) => p.id === nextKind)
    if (desc) {
      const sameAsSaved = config?.provider?.kind === nextKind
      const nextUrl = sameAsSaved
        ? config?.provider?.base_url || desc.default_base_url
        : desc.default_base_url
      setBaseUrl(nextUrl)
      setLeadModel(sameAsSaved ? config?.provider?.lead_model || '' : '')
      setFastModel(sameAsSaved ? config?.provider?.fast_model || '' : '')
    }
  }

  const save = async () => {
    setSaving(true)
    try {
      const resolvedLead = leadModel.trim() || discoveredModels[0]?.id || ''
      const resolvedFast = fastModel.trim() || resolvedLead
      await api('/api/setup', {
        method: 'POST',
        body: JSON.stringify({
          provider_kind: providerKind,
          api_key: apiKey.trim() || null,
          base_url: baseUrl.trim(),
          lead_model: resolvedLead,
          fast_model: resolvedFast,
          fallbacks: fallbacks.split(',').map((s: string) => s.trim()).filter(Boolean),
          telegram_bot_token: tgToken.trim() || null,
          telegram_enabled: tgEnabled,
          workspace_dir: workspaceDir.trim(),
          daily_spend_limit_usd: Number(budget || 5),
          policy_mode: policyMode,
        }),
      })
      toast(`Saved ${activeDesc?.label || providerKind} to ~/.zenko/zenko.toml & vault.enc`, 'success')
      onSaved()
    } catch (e: any) {
      toast(e.message, 'error')
    } finally {
      setSaving(false)
    }
  }

  return (
    <ScrollPage smoothScroll={smoothScroll}>
      <div className="mx-auto flex w-full max-w-[860px] flex-col gap-6">
        <div>
          <h1 className="text-[22px] font-semibold tracking-[-0.03em]" style={{ fontFamily: 'var(--font-display)' }}>
            Settings & Setup
          </h1>
          <p className="mt-1 text-[13px] text-[var(--muted)]">
            Secrets are encrypted at rest with ChaCha20-Poly1305 and never exposed to shell tools or LLM context.
          </p>
        </div>

        <div className="grid gap-5 md:grid-cols-2">
          <div className="z-surface-card flex flex-col gap-3">
            <div className="flex items-center justify-between">
              <strong className="text-[14px]">{activeDesc?.label || 'LLM'} Provider</strong>
              <span
                className="rounded-full border px-2 py-0.5 font-mono text-[11px]"
                style={
                  !needsKey || hasSelectedKey
                    ? {
                        color: 'var(--muted)',
                        borderColor: 'var(--border)',
                        backgroundColor: 'var(--surface)',
                      }
                    : {
                        color: 'var(--accent)',
                        borderColor: 'color-mix(in srgb, var(--accent) 38%, transparent)',
                        backgroundColor: 'color-mix(in srgb, var(--accent) 10%, transparent)',
                      }
                }
              >
                {!needsKey ? 'No Key Needed' : hasSelectedKey ? 'Vault Key Set' : 'Key Required'}
              </span>
            </div>

            <div className="flex flex-col gap-1 text-[11.5px] text-[var(--muted)]">
              <span>Provider</span>
              <ZenkoSelect
                ariaLabel="LLM Provider"
                value={providerKind}
                onChange={selectProvider}
                options={providers.map((p) => ({
                  value: p.id,
                  label: `${p.label}${configuredProviders[p.id] && p.requires_api_key ? ' · Key Set' : ''}`,
                }))}
              />
            </div>
            {activeDesc && (
              <p className="-mt-1 text-[11.5px] text-[var(--muted)]">{activeDesc.description}</p>
            )}

            <InputField
              label={needsKey ? 'API Key (blank keeps existing)' : 'API Key (optional)'}
              type="password"
              value={apiKey}
              onChange={setApiKey}
              placeholder={activeDesc?.vault_secret_key || 'Paste API key...'}
              mono
            />
            <InputField label="Base URL" value={baseUrl} onChange={setBaseUrl} mono />
            <InputField
              label="Lead Model (auto-discovered if blank)"
              value={leadModel}
              onChange={setLeadModel}
              placeholder="Pick live model below or paste ID..."
              mono
            />
            <InputField
              label="Fast / Subagent Model"
              value={fastModel}
              onChange={setFastModel}
              placeholder="Optional faster model ID..."
              mono
            />

            <div className="flex flex-col gap-1.5">
              <div className="flex items-center justify-between text-[11px] text-[var(--muted)]">
                <span>
                  Live Working Models ({discoveredModels.length}) · Free First
                </span>
                <button
                  type="button"
                  onClick={() => loadLiveModels(providerKind, baseUrl, true)}
                  className="inline-flex cursor-pointer items-center gap-1 font-mono text-[10.5px] text-[var(--muted)] hover:text-[var(--fg)]"
                >
                  <RefreshCw size={10.5} className={loadingModels ? 'animate-spin' : ''} /> Refresh
                </button>
              </div>
              <CompactScrollArea
                autoHeight
                className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)]"
                viewportClassName="max-h-36 p-2 pr-3"
              >
                <div className="flex flex-wrap gap-1.5">
                {loadingModels ? (
                  <span className="font-mono text-[11px] text-[var(--muted)]">
                    Fetching live models & context windows...
                  </span>
                ) : discoveredModels.length === 0 ? (
                  <span className="font-mono text-[11px] text-[var(--muted)]">
                    No live models fetched yet — verify key or click Refresh.
                  </span>
                ) : (
                  discoveredModels.slice(0, 24).map((m) => (
                    <button
                      key={m.id}
                      type="button"
                      onClick={() => setLeadModel(m.id)}
                      className={
                        leadModel === m.id
                          ? 'z-btn z-btn-vermilion-soft z-btn-sm font-mono text-[10.5px]'
                          : 'z-btn z-btn-ghost z-btn-sm font-mono text-[10.5px]'
                      }
                    >
                      <span>{m.id}</span>
                      {m.is_free && (
                        <span className="ml-1 font-semibold" style={{ color: 'var(--accent)' }}>
                          FREE
                        </span>
                      )}
                      {m.context_length != null && (
                        <span className="ml-1 text-[var(--muted)]">
                          {formatCompactTokens(m.context_length)}
                        </span>
                      )}
                    </button>
                  ))
                )}
                </div>
              </CompactScrollArea>
            </div>

            {providerKind === 'openrouter' && (
              <InputField label="Fallbacks (comma-separated)" value={fallbacks} onChange={setFallbacks} mono />
            )}
            <div className="flex items-center gap-2">
              <button
                type="button"
                className="z-btn z-btn-secondary z-btn-sm"
                onClick={async () => {
                  setProviderStatus('Verifying...')
                  try {
                    const res = await api('/api/setup/verify-provider', {
                      method: 'POST',
                      body: JSON.stringify({
                        provider_kind: providerKind,
                        api_key: apiKey.trim() || null,
                        base_url: baseUrl.trim(),
                      }),
                    })
                    setProviderStatus(`Connected · ${res.model_count || 0} models`)
                    loadLiveModels(providerKind, baseUrl, true)
                  } catch (e: any) {
                    setProviderStatus(e.message)
                  }
                }}
              >
                Verify Key
              </button>
              {providerStatus && (
                <span className="font-mono text-[11.5px] text-[var(--muted)]">{providerStatus}</span>
              )}
            </div>
          </div>

          <div className="z-surface-card flex flex-col gap-3">
            <div className="flex items-center justify-between">
              <strong className="text-[14px]">Telegram, Workspace & Policy</strong>
              <span className="font-mono text-[11px] text-[var(--muted)]">
                {hasTelegram ? 'TG Token Set' : 'Optional'}
              </span>
            </div>
            <InputField label="Telegram Bot Token (@BotFather)" type="password" value={tgToken} onChange={setTgToken} placeholder="123456:AAH..." mono />
            <label className="flex cursor-pointer items-center gap-2 text-[13px]">
              <input type="checkbox" checked={tgEnabled} onChange={(e) => setTgEnabled(e.target.checked)} className="accent-[var(--accent)]" />
              <span>Enable 24/7 Telegram Gateway</span>
            </label>
            <div className="flex items-center gap-2">
              <button
                type="button"
                className="z-btn z-btn-secondary z-btn-sm"
                onClick={async () => {
                  setTgStatus('Verifying...')
                  try {
                    const res = await api('/api/setup/verify-telegram', {
                      method: 'POST',
                      body: JSON.stringify({ token: tgToken.trim() || null }),
                    })
                    setTgStatus(`@${res.username || res.first_name}`)
                  } catch (e: any) {
                    setTgStatus(e.message)
                  }
                }}
              >
                Test Bot
              </button>
              {tgStatus && <span className="font-mono text-[11.5px] text-[var(--muted)]">{tgStatus}</span>}
            </div>

            <div className="my-1 h-px bg-[var(--border)]" />
            <InputField label="Workspace Directory" value={workspaceDir} onChange={setWorkspaceDir} mono />
            <div className="grid grid-cols-2 gap-3">
              <InputField label="Daily Cap (USD)" value={budget} onChange={setBudget} mono />
              <div className="flex flex-col gap-1 text-[11.5px] text-[var(--muted)]">
                <span>Execution Policy</span>
                <ZenkoSelect
                  ariaLabel="Execution Policy"
                  value={policyMode}
                  onChange={setPolicyMode}
                  options={[
                    { value: 'autonomous', label: 'Autonomous', hint: 'Full freedom + dangerous-command guard' },
                    { value: 'balanced', label: 'Balanced', hint: 'Ask before shell commands' },
                    { value: 'strict', label: 'Strict', hint: 'Ask before writes & shell' },
                    { value: 'plan', label: 'Plan', hint: 'Read-only inspection' },
                  ]}
                />
              </div>
            </div>
            <button type="button" disabled={saving} onClick={save} className="z-btn z-btn-primary mt-auto w-full py-2.5">
              <Check size={14} /> {saving ? 'Saving...' : 'Save Configuration'}
            </button>
          </div>
        </div>

        {/* Phase 7: Pluggable Web Search, External MCP Client Servers & Pooled LSP */}
        <div className="grid gap-5 md:grid-cols-2">
          <div className="z-surface-card flex flex-col gap-3">
            <div className="flex items-center justify-between">
              <strong className="text-[14px]">Web Search (web_search)</strong>
              <label className="flex cursor-pointer items-center gap-1.5 font-mono text-[11px] text-[var(--muted)]">
                <input
                  type="checkbox"
                  checked={searchEnabled}
                  onChange={(e) => setSearchEnabled(e.target.checked)}
                  className="accent-[var(--accent)]"
                />
                <span>{searchEnabled ? 'Enabled' : 'Disabled'}</span>
              </label>
            </div>
            <div className="flex flex-col gap-1 text-[11.5px] text-[var(--muted)]">
              <span>Backend Cascade</span>
              <ZenkoSelect
                ariaLabel="Web Search Backend Cascade"
                value={searchProvider}
                onChange={setSearchProvider}
                options={webSearchProviders.map((p) => ({ value: p.id, label: p.label }))}
              />
            </div>
            <InputField
              label="SearXNG Base URL (optional)"
              value={searxngUrl}
              onChange={setSearxngUrl}
              placeholder="http://127.0.0.1:8080"
              mono
            />
            <label className="flex cursor-pointer items-center gap-2 text-[12px] text-[var(--muted)]">
              <input
                type="checkbox"
                checked={allowPrivateSearxng}
                onChange={(e) => setAllowPrivateSearxng(e.target.checked)}
                className="accent-[var(--accent)]"
              />
              <span>Allow local/LAN SearXNG URL (SSRF opt-in)</span>
            </label>
            <div className="grid grid-cols-2 gap-3">
              <InputField
                label={hasTavilySearchKey ? 'Tavily Key (Set)' : 'Tavily API Key'}
                type="password"
                value={tavilyKey}
                onChange={setTavilyKey}
                placeholder="tvly-..."
                mono
              />
              <InputField
                label={hasBraveSearchKey ? 'Brave Key (Set)' : 'Brave Search Key'}
                type="password"
                value={braveKey}
                onChange={setBraveKey}
                placeholder="BSA..."
                mono
              />
            </div>
            <button
              type="button"
              disabled={savingSearch}
              onClick={async () => {
                setSavingSearch(true)
                try {
                  await api('/api/web-search', {
                    method: 'PUT',
                    body: JSON.stringify({
                      enabled: searchEnabled,
                      provider: searchProvider,
                      searxng_url: searxngUrl,
                      allow_private_searxng: allowPrivateSearxng,
                      tavily_api_key: tavilyKey.trim() || null,
                      brave_api_key: braveKey.trim() || null,
                    }),
                  })
                  setTavilyKey('')
                  setBraveKey('')
                  toast('Saved web search settings', 'success')
                  onRefreshBootstrap?.()
                } catch (e: any) {
                  toast(e.message, 'error')
                } finally {
                  setSavingSearch(false)
                }
              }}
              className="z-btn z-btn-secondary z-btn-sm mt-auto w-full"
            >
              <Check size={13} /> {savingSearch ? 'Saving...' : 'Save Web Search'}
            </button>
          </div>

          <div className="z-surface-card flex flex-col gap-3">
            <div className="flex items-center justify-between">
              <strong className="text-[14px]">External MCP Client ({mcpServers.length})</strong>
              <button
                type="button"
                onClick={() => setMcpAddOpen((o) => !o)}
                className="z-btn z-btn-secondary z-btn-sm"
              >
                {mcpAddOpen ? <X size={12} /> : <Plus size={12} />}
                <span>{mcpAddOpen ? 'Cancel' : 'Add Server'}</span>
              </button>
            </div>
            <p className="-mt-1 text-[11.5px] text-[var(--muted)]">
              Connect stdio or Streamable HTTP MCP servers. Tools register lazily as deferred <code className="font-mono">mcp__&lt;server&gt;__*</code> schemas.
            </p>

            {mcpAddOpen && (
              <div className="flex flex-col gap-2.5 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] p-3">
                <div className="grid grid-cols-2 gap-2.5">
                  <InputField label="Server Name" value={mcpName} onChange={setMcpName} placeholder="github" mono />
                  <div className="flex flex-col gap-1 text-[11.5px] text-[var(--muted)]">
                    <span>Transport</span>
                    <ZenkoSelect
                      ariaLabel="MCP Transport"
                      value={mcpTransport}
                      onChange={(v) => setMcpTransport(v as 'stdio' | 'http')}
                      className="bg-[var(--surface)]"
                      options={[
                        { value: 'stdio', label: 'stdio', hint: 'Local process' },
                        { value: 'http', label: 'http', hint: 'Streamable 2025-03-26' },
                      ]}
                    />
                  </div>
                </div>
                {mcpTransport === 'stdio' ? (
                  <div className="grid grid-cols-2 gap-2.5">
                    <InputField label="Command" value={mcpCommand} onChange={setMcpCommand} placeholder="npx" mono />
                    <InputField label="Args (space-separated)" value={mcpArgs} onChange={setMcpArgs} placeholder="-y @modelcontextprotocol/server-..." mono />
                  </div>
                ) : (
                  <div className="grid grid-cols-2 gap-2.5">
                    <InputField label="Endpoint URL" value={mcpUrl} onChange={setMcpUrl} placeholder="https://mcp.example.com/mcp" mono />
                    <InputField label="Auth Vault Key (opt)" value={mcpAuthSecret} onChange={setMcpAuthSecret} placeholder="MCP_TOKEN" mono />
                  </div>
                )}
                <div className="flex flex-wrap items-center gap-3 text-[11.5px] text-[var(--muted)]">
                  <div className="w-48">
                    <ZenkoSelect
                      ariaLabel="MCP Tool Risk Level"
                      value={mcpRisk}
                      onChange={setMcpRisk}
                      className="bg-[var(--surface)] py-1 text-[11.5px]"
                      options={[
                        { value: '', label: 'Risk: Moderate (default)' },
                        { value: 'safe', label: 'Risk: Safe (Plan Mode ok)' },
                        { value: 'dangerous', label: 'Risk: Dangerous (HITL)' },
                      ]}
                    />
                  </div>
                  <label className="flex cursor-pointer items-center gap-1.5">
                    <input type="checkbox" checked={mcpParallel} onChange={(e) => setMcpParallel(e.target.checked)} className="accent-[var(--accent)]" />
                    <span>Parallel</span>
                  </label>
                  {mcpTransport === 'http' && (
                    <label className="flex cursor-pointer items-center gap-1.5">
                      <input type="checkbox" checked={mcpAllowPrivate} onChange={(e) => setMcpAllowPrivate(e.target.checked)} className="accent-[var(--accent)]" />
                      <span>Local URL</span>
                    </label>
                  )}
                </div>
                <button
                  type="button"
                  disabled={!mcpName.trim() || mcpBusy === '__add__'}
                  onClick={async () => {
                    setMcpBusy('__add__')
                    try {
                      const res = await api<{ servers: McpServerStatus[]; sync_error?: string | null }>('/api/mcp/servers', {
                        method: 'POST',
                        body: JSON.stringify({
                          name: mcpName.trim(),
                          enabled: true,
                          transport: mcpTransport,
                          command: mcpCommand.trim(),
                          args: splitShellArgs(mcpArgs),
                          url: mcpUrl.trim(),
                          auth_header_secret: mcpAuthSecret.trim() || null,
                          risk_override: mcpRisk || null,
                          supports_parallel: mcpParallel,
                          allow_private_url: mcpAllowPrivate,
                          sync: true,
                        }),
                      })
                      onMcpUpdated?.(res.servers || [])
                      setMcpAddOpen(false)
                      setMcpName('')
                      setMcpCommand('')
                      setMcpArgs('')
                      setMcpUrl('')
                      setMcpAuthSecret('')
                      toast(
                        res.sync_error
                          ? `Saved MCP server (sync warning: ${res.sync_error})`
                          : 'Saved & synced MCP server',
                        res.sync_error ? 'info' : 'success',
                      )
                    } catch (e: any) {
                      toast(e.message, 'error')
                    } finally {
                      setMcpBusy(null)
                    }
                  }}
                  className="z-btn z-btn-primary z-btn-sm w-full"
                >
                  {mcpBusy === '__add__' ? 'Syncing...' : 'Save & Sync Tools'}
                </button>
              </div>
            )}

            <CompactScrollArea autoHeight viewportClassName="max-h-52 pr-2.5">
              <div className="flex flex-col gap-1.5">
              {mcpServers.length === 0 ? (
                <div className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2.5 font-mono text-[11.5px] text-[var(--muted)]">
                  No external MCP servers configured yet.
                </div>
              ) : (
                mcpServers.map((srv) => (
                  <div
                    key={srv.name}
                    className="flex flex-col gap-1.5 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <button
                        type="button"
                        onClick={() => setExpandedMcp(expandedMcp === srv.name ? null : srv.name)}
                        className="flex min-w-0 cursor-pointer items-center gap-2 text-left"
                      >
                        <span className="truncate font-mono text-[12px] font-medium text-[var(--fg)]">
                          {srv.name}
                        </span>
                        <span className="rounded border border-[var(--border)] bg-[var(--surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--muted)]">
                          {srv.transport}
                        </span>
                        <span className="font-mono text-[10.5px] text-[var(--muted)]">
                          {srv.enabled ? `${srv.tool_count} tool${srv.tool_count === 1 ? '' : 's'}` : 'disabled'}
                          {srv.catalog_dirty ? ' · dirty' : ''}
                        </span>
                      </button>
                      <div className="flex items-center gap-1">
                        <button
                          type="button"
                          disabled={mcpBusy === srv.name}
                          onClick={async () => {
                            setMcpBusy(srv.name)
                            try {
                              const res = await api<{ servers: McpServerStatus[]; sync_error?: string | null }>(
                                `/api/mcp/servers/${encodeURIComponent(srv.name)}/toggle`,
                                { method: 'POST', body: JSON.stringify({ enabled: !srv.enabled }) },
                              )
                              onMcpUpdated?.(res.servers || [])
                              toast(
                                res.sync_error
                                  ? `Updated ${srv.name} (sync warning: ${res.sync_error})`
                                  : `${srv.name} ${srv.enabled ? 'disabled' : 'enabled'}`,
                                res.sync_error ? 'info' : 'success',
                              )
                            } catch (e: any) {
                              toast(e.message, 'error')
                            } finally {
                              setMcpBusy(null)
                            }
                          }}
                          className="z-btn z-btn-ghost z-btn-sm px-1.5 font-mono text-[10px]"
                          title={srv.enabled ? 'Disable server' : 'Enable server'}
                        >
                          {srv.enabled ? 'On' : 'Off'}
                        </button>
                        <button
                          type="button"
                          disabled={mcpBusy === srv.name}
                          onClick={async () => {
                            setMcpBusy(srv.name)
                            try {
                              const res = await api<{ servers: McpServerStatus[] }>(
                                `/api/mcp/servers/${encodeURIComponent(srv.name)}/sync`,
                                { method: 'POST' },
                              )
                              onMcpUpdated?.(res.servers || [])
                              toast(`Synced ${srv.name}`, 'success')
                            } catch (e: any) {
                              toast(e.message, 'error')
                            } finally {
                              setMcpBusy(null)
                            }
                          }}
                          className="z-btn z-btn-ghost z-btn-sm px-1.5"
                          title="Sync tools/list"
                        >
                          <RefreshCw size={12} className={mcpBusy === srv.name ? 'animate-spin' : ''} />
                        </button>
                        <button
                          type="button"
                          disabled={mcpBusy === srv.name}
                          onClick={async () => {
                            setMcpBusy(srv.name)
                            try {
                              const res = await api<{ servers: McpServerStatus[] }>(
                                `/api/mcp/servers/${encodeURIComponent(srv.name)}`,
                                { method: 'DELETE' },
                              )
                              onMcpUpdated?.(res.servers || [])
                              toast(`Removed ${srv.name}`, 'info')
                            } catch (e: any) {
                              toast(e.message, 'error')
                            } finally {
                              setMcpBusy(null)
                            }
                          }}
                          className="z-btn z-btn-ghost z-btn-sm px-1.5 text-[var(--muted)] hover:text-[var(--fg)]"
                          title="Remove server"
                        >
                          <Trash2 size={12} />
                        </button>
                      </div>
                    </div>
                    {srv.last_error && (
                      <div className="font-mono text-[10.5px] text-[var(--accent)]">
                        {srv.last_error}
                      </div>
                    )}
                    {expandedMcp === srv.name && srv.tools.length > 0 && (
                      <div className="flex flex-wrap gap-1 border-t border-[var(--border)] pt-1.5">
                        {srv.tools.map((t) => (
                          <span
                            key={t}
                            className="rounded border border-[var(--border)] bg-[var(--surface)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--muted)]"
                          >
                            {t}
                          </span>
                        ))}
                      </div>
                    )}
                  </div>
                ))
              )}
              </div>
            </CompactScrollArea>

            <div className="mt-auto flex items-center justify-between border-t border-[var(--border)] pt-2.5 text-[11px] text-[var(--muted)]">
              <span className="font-mono">
                LSP Pool: {lspActive.length} active · {lspRegistry.length || 5} built-in langs
              </span>
              {lspActive.length > 0 && (
                <button
                  type="button"
                  onClick={async () => {
                    try {
                      await api('/api/lsp/stop', { method: 'POST' })
                      onLspUpdated?.([])
                      toast('Stopped pooled LSP servers', 'info')
                    } catch (e: any) {
                      toast(e.message, 'error')
                    }
                  }}
                  className="z-btn z-btn-ghost z-btn-sm font-mono text-[10.5px]"
                >
                  Stop Pool
                </button>
              )}
            </div>
          </div>
        </div>
      </div>
    </ScrollPage>
  )
}

// ─── Gateways View ────────────────────────────────────────────────────────────

export function GatewaysView({
  toast,
  smoothScroll = true,
}: {
  toast: (msg: string, kind?: 'info' | 'success' | 'error') => void
  smoothScroll?: boolean
}) {
  const [data, setData] = useState<{
    config: any
    connected: boolean
    peers: GatewayPeer[]
    pairing_codes: PairingCode[]
  } | null>(null)
  const [token, setToken] = useState('')
  const [role, setRole] = useState('owner')

  const load = async () => {
    try {
      setData(await api('/api/gateways/telegram'))
    } catch (e: any) {
      toast(e.message, 'error')
    }
  }
  useEffect(() => { load() }, [])

  return (
    <ScrollPage smoothScroll={smoothScroll}>
      <div className="mx-auto flex w-full max-w-[860px] flex-col gap-6">
        <div className="flex items-center justify-between">
          <div>
            <h1 className="text-[22px] font-semibold tracking-[-0.03em]" style={{ fontFamily: 'var(--font-display)' }}>
              Telegram Gateway
            </h1>
            <p className="mt-1 text-[13px] text-[var(--muted)]">
              Zero-trust pairing codes, Forum Topic session isolation, live draft streaming, and inline keyboard approvals.
            </p>
          </div>
          <span
            className="inline-flex items-center gap-1.5 rounded-full border bg-[var(--surface)] px-3 py-1 font-mono text-[11.5px]"
            style={{
              color: data?.connected ? 'var(--fg)' : 'var(--muted)',
              borderColor: data?.connected
                ? 'color-mix(in srgb, var(--accent) 42%, var(--border))'
                : 'var(--border)',
            }}
          >
            {data?.connected ? `Connected @${data?.config?.bot_username || 'bot'}` : 'Offline'}
          </span>
        </div>

        <div className="grid gap-5 md:grid-cols-2">
          <div className="z-surface-card flex flex-col gap-3">
            <strong className="text-[14px]">Bot Connection</strong>
            <InputField label="New @BotFather Token (optional)" type="password" value={token} onChange={setToken} placeholder="Leave blank to keep stored token" mono />
            <label className="flex cursor-pointer items-center gap-2 text-[13px]">
              <input
                type="checkbox"
                checked={Boolean(data?.config?.enabled)}
                onChange={(e) => setData((d) => (d ? { ...d, config: { ...d.config, enabled: e.target.checked } } : d))}
                className="accent-[var(--accent)]"
              />
              <span>Gateway Enabled</span>
            </label>
            <label className="flex cursor-pointer items-center gap-2 text-[13px]">
              <input
                type="checkbox"
                checked={Boolean(data?.config?.stream_drafts)}
                onChange={(e) => setData((d) => (d ? { ...d, config: { ...d.config, stream_drafts: e.target.checked } } : d))}
                className="accent-[var(--accent)]"
              />
              <span>Live Token Draft Streaming</span>
            </label>
            <button
              type="button"
              className="z-btn z-btn-primary"
              onClick={async () => {
                try {
                  await api('/api/gateways/telegram', {
                    method: 'POST',
                    body: JSON.stringify({
                      token: token.trim() || null,
                      enabled: Boolean(data?.config?.enabled),
                      stream_drafts: Boolean(data?.config?.stream_drafts),
                    }),
                  })
                  setToken('')
                  toast('Telegram settings updated', 'success')
                  load()
                } catch (e: any) {
                  toast(e.message, 'error')
                }
              }}
            >
              Save Gateway
            </button>
          </div>

          <div className="z-surface-card flex flex-col gap-3">
            <strong className="text-[14px]">One-Time Pairing Code</strong>
            <p className="text-[12.5px] text-[var(--muted)]">
              Generate a 6-character code, then message <code className="font-mono">/pair CODE</code> to your bot.
            </p>
            <div className="flex gap-2">
              <div className="min-w-0 flex-1">
                <ZenkoSelect
                  ariaLabel="Telegram Pairing Role"
                  value={role}
                  onChange={setRole}
                  options={[
                    { value: 'owner', label: 'Owner (Full + Approvals)' },
                    { value: 'trusted', label: 'Trusted' },
                    { value: 'readonly', label: 'Read-Only' },
                  ]}
                />
              </div>
              <button
                type="button"
                className="z-btn z-btn-primary shrink-0"
                onClick={async () => {
                  const res = await api('/api/gateways/telegram/pair', {
                    method: 'POST',
                    body: JSON.stringify({ role }),
                  })
                  toast(`Pairing code: ${res.code}`, 'success')
                  load()
                }}
              >
                Generate
              </button>
            </div>
            {(data?.pairing_codes || []).map((c) => (
              <div
                key={c.code}
                className="flex items-center justify-between rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 font-mono text-[12px]"
              >
                <span>
                  /pair <strong style={{ color: 'var(--accent)' }}>{c.code}</strong>
                </span>
                <span className="text-[var(--muted)]">{c.role}</span>
              </div>
            ))}
          </div>
        </div>

        <div className="z-surface-card flex flex-col gap-3">
          <strong className="text-[14px]">Paired Accounts ({data?.peers.length || 0})</strong>
          {(data?.peers || []).length === 0 ? (
            <p className="text-[13px] text-[var(--muted)]">No paired Telegram users yet.</p>
          ) : (
            data?.peers.map((p) => (
              <div
                key={p.peer_id}
                className="flex items-center justify-between rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3.5 py-2.5 text-[13px]"
              >
                <div>
                  <span className="font-medium">{p.display_name || 'User'}</span>
                  {p.username && <span className="ml-1.5 text-[var(--muted)]">@{p.username}</span>}
                  <span className="ml-2 font-mono text-[11px] text-[var(--muted)]">#{p.peer_id}</span>
                </div>
                <div className="flex items-center gap-2">
                  <span className="rounded-full border border-[var(--border)] px-2 py-0.5 font-mono text-[11px]">
                    {p.role}
                  </span>
                  <button
                    type="button"
                    className="z-btn z-btn-danger z-btn-sm"
                    onClick={async () => {
                      await api(`/api/gateways/telegram/peers/${encodeURIComponent(p.peer_id)}`, { method: 'DELETE' })
                      load()
                    }}
                  >
                    Revoke
                  </button>
                </div>
              </div>
            ))
          )}
        </div>
      </div>
    </ScrollPage>
  )
}

// ─── Memory & Skills View ─────────────────────────────────────────────────────

const MEMORY_DOC_NAMES = ['SOUL.md', 'USER.md', 'MEMORY.md', 'PROJECT_MEMORY.md', 'ZENKO.md'] as const
type MemoryDocName = (typeof MEMORY_DOC_NAMES)[number]

const MEMORY_SCOPE_FILTERS = [
  { id: 'all', label: 'All' },
  { id: 'current', label: 'Current' },
  { id: 'project', label: 'Project' },
  { id: 'global', label: 'Global' },
] as const
type MemoryScopeFilter = (typeof MEMORY_SCOPE_FILTERS)[number]['id']

const SESSION_SINCE_FILTERS = [
  { id: '24h', label: '24h' },
  { id: '7d', label: '7d' },
  { id: '30d', label: '30d' },
  { id: '', label: 'All' },
] as const

const FALLBACK_MEMORY_KINDS: MemoryKindMeta[] = [
  { id: 'project', label: 'Project', description: 'Workspace architecture decisions', half_life_days: 30 },
  { id: 'fact', label: 'Fact', description: 'Verified technical facts', half_life_days: 30 },
  { id: 'user', label: 'User', description: 'Developer habits & preferences', half_life_days: 120 },
  { id: 'feedback', label: 'Feedback', description: 'Explicit corrections & rules', half_life_days: 120 },
  { id: 'lesson', label: 'Lesson', description: 'Bug root causes & takeaways', half_life_days: 45 },
  { id: 'reference', label: 'Reference', description: 'Links, endpoints & pointers', half_life_days: 60 },
]

function getDocText(name: MemoryDocName, s: WorkspaceSnapshot): string {
  switch (name) {
    case 'SOUL.md':
      return s.soul_md
    case 'USER.md':
      return s.user_md
    case 'MEMORY.md':
      return s.memory_md
    case 'PROJECT_MEMORY.md':
      return s.project_memory_md ?? ''
    case 'ZENKO.md':
      return s.zenko_md
  }
}

export function MemoryView({
  toast,
  tabsVariant = 'pill',
  smoothScroll = true,
}: {
  toast: (msg: string, kind?: 'info' | 'success' | 'error') => void
  tabsVariant?: SecondaryTabsVariant
  smoothScroll?: boolean
}) {
  const [snap, setSnap] = useState<WorkspaceSnapshot | null>(null)
  const [hits, setHits] = useState<ScoredMemoryHit[]>([])
  const [memoryKinds, setMemoryKinds] = useState<MemoryKindMeta[]>(FALLBACK_MEMORY_KINDS)
  const [dream, setDream] = useState<DreamStatus | null>(null)
  const [history, setHistory] = useState<MemoryDocSnapshot[]>([])
  const [skillHistory, setSkillHistory] = useState<SkillVersionSnapshot[]>([])
  const [showSkillHistory, setShowSkillHistory] = useState(false)
  const [newSkillSlug, setNewSkillSlug] = useState('')
  const [newSkillBody, setNewSkillBody] = useState('')
  const [projectScope, setProjectScope] = useState<string>('')
  const [activeDoc, setActiveDoc] = useState<MemoryDocName>('MEMORY.md')
  const [docContent, setDocContent] = useState('')
  const [showHistory, setShowHistory] = useState(false)
  const [dreamRunning, setDreamRunning] = useState(false)
  const [query, setQuery] = useState('')
  const [scopeFilter, setScopeFilter] = useState<MemoryScopeFilter>('all')
  const [newTitle, setNewTitle] = useState('')
  const [newContent, setNewContent] = useState('')
  const [newKind, setNewKind] = useState<string>('project')
  const [newScope, setNewScope] = useState<'global' | 'project'>('project')
  const [editingId, setEditingId] = useState<string | null>(null)
  const [editTitle, setEditTitle] = useState('')
  const [editContent, setEditContent] = useState('')

  const [sessionQuery, setSessionQuery] = useState('')
  const [sessionSince, setSessionSince] = useState<string>('7d')
  const [sessionWs, setSessionWs] = useState<'current' | 'all'>('current')
  const [sessionHits, setSessionHits] = useState<SessionSearchHit[]>([])
  const [sessionSearching, setSessionSearching] = useState(false)
  const docDirtyRef = useRef<boolean>(false)
  const loadSeqRef = useRef<number>(0)
  const loadAbortRef = useRef<AbortController | null>(null)
  const searchDebounceRef = useRef<number | undefined>(undefined)

  const pickDoc = (name: MemoryDocName, s: WorkspaceSnapshot) => {
    docDirtyRef.current = false
    setActiveDoc(name)
    setDocContent(getDocText(name, s))
  }

  const load = async (
    q = query,
    sc: MemoryScopeFilter = scopeFilter,
    preserveDoc = activeDoc,
    opts?: { forceSyncDoc?: boolean },
  ) => {
    const seq = ++loadSeqRef.current
    loadAbortRef.current?.abort()
    const controller = new AbortController()
    loadAbortRef.current = controller
    const params = new URLSearchParams()
    if (q.trim()) params.set('q', q.trim())
    if (sc !== 'all') params.set('scope', sc)
    const qs = params.toString()
    try {
      const res = await api<{
        snapshot: WorkspaceSnapshot
        entries: MemoryEntry[]
        hits?: ScoredMemoryHit[]
        memory_kinds?: MemoryKindMeta[]
        project_scope?: string
        dream?: DreamStatus
        history?: MemoryDocSnapshot[]
        skill_history?: SkillVersionSnapshot[]
      }>(`/api/memory${qs ? `?${qs}` : ''}`, { signal: controller.signal })
      if (seq !== loadSeqRef.current) return
      setSnap(res.snapshot)
      setHits(
        res.hits ??
          res.entries.map((entry) => ({
            entry,
            score: 0,
            stale_paths: [],
          })),
      )
      if (res.memory_kinds?.length) setMemoryKinds(res.memory_kinds)
      if (res.project_scope) setProjectScope(res.project_scope)
      if (res.dream) setDream(res.dream)
      if (res.history) setHistory(res.history)
      if (res.skill_history) setSkillHistory(res.skill_history)
      if (opts?.forceSyncDoc || !docDirtyRef.current) {
        pickDoc(preserveDoc, res.snapshot)
      }
    } catch (e: any) {
      if (e?.name === 'AbortError') return
      toast(e?.message || 'Failed to load memory', 'error')
    }
  }

  const searchSessions = async (q = sessionQuery, since = sessionSince, ws = sessionWs) => {
    if (!q.trim()) {
      setSessionHits([])
      return
    }
    setSessionSearching(true)
    try {
      const params = new URLSearchParams({ q: q.trim(), workspace: ws, limit: '10' })
      if (since) params.set('since', since)
      const res = await api<{ hits: SessionSearchHit[] }>(`/api/sessions/search?${params.toString()}`)
      setSessionHits(res.hits ?? [])
    } catch {
      setSessionHits([])
    } finally {
      setSessionSearching(false)
    }
  }

  useEffect(() => {
    load('', 'all', 'MEMORY.md', { forceSyncDoc: true })
    return () => {
      if (searchDebounceRef.current) window.clearTimeout(searchDebounceRef.current)
      loadAbortRef.current?.abort()
    }
  }, [])

  const activeDocHistory = history.filter((h) => h.doc_name === activeDoc)
  const lastDreamRun = dream?.recent_runs?.[0] ?? null

  return (
    <ScrollPage smoothScroll={smoothScroll}>
      <div className="mx-auto flex w-full max-w-[900px] flex-col gap-6">
        <div className="flex flex-wrap items-end justify-between gap-3">
          <div>
            <h1 className="text-[22px] font-semibold tracking-[-0.03em]" style={{ fontFamily: 'var(--font-display)' }}>
              Memory & Prompt Snapshots
            </h1>
            <p className="mt-1 text-[13px] text-[var(--muted)]">
              Frozen KV-cache Markdown prefix ({snap?.total_tokens_estimate ?? 0} est. tokens) + scoped RRF episodic index.
            </p>
          </div>
          {projectScope && (
            <span className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1 font-mono text-[11px] text-[var(--muted)]">
              {projectScope}
            </span>
          )}
        </div>

        {/* Autonomous Dream Consolidation Status */}
        <div className="z-surface-card flex flex-wrap items-center justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <BrainIcon size={14} className="text-[var(--accent)]" />
              <strong className="text-[13.5px]">Autonomous Dream Consolidation</strong>
              <span className="rounded border border-[var(--border)] px-1.5 py-0.5 font-mono text-[10.5px] text-[var(--muted)]">
                {dream?.running
                  ? 'running'
                  : dream?.enabled === false
                    ? 'disabled'
                    : `${dream?.pending_sessions ?? 0} pending`}
              </span>
            </div>
            <p className="mt-1 truncate text-[12px] text-[var(--muted)]">
              {lastDreamRun
                ? `Last (${lastDreamRun.status}${lastDreamRun.model ? ` · ${lastDreamRun.model}` : ''}): +${lastDreamRun.memories_added} added, ${lastDreamRun.memories_merged} merged, ${lastDreamRun.memories_archived} archived, ${lastDreamRun.docs_patched} docs`
                : 'Runs automatically during idle windows: SimHash hygiene, contradiction archival & reversible doc synthesis.'}
            </p>
          </div>
          <button
            type="button"
            disabled={dreamRunning || dream?.running}
            className="z-btn z-btn-vermilion-soft z-btn-sm shrink-0"
            onClick={async () => {
              setDreamRunning(true)
              try {
                const rec = await api<DreamRunRecord>('/api/memory/dream', {
                  method: 'POST',
                  body: JSON.stringify({ force: true }),
                })
                toast(
                  `Dream ${rec.status}: +${rec.memories_added} added, ${rec.memories_archived} archived, ${rec.docs_patched} docs`,
                  rec.status === 'failed' ? 'error' : 'success',
                )
                await load(query, scopeFilter, activeDoc)
              } catch (e: any) {
                toast(e.message || 'Dream run failed', 'error')
              } finally {
                setDreamRunning(false)
              }
            }}
          >
            <RefreshCw size={12.5} className={dreamRunning ? 'animate-spin' : ''} />
            {dreamRunning ? 'Consolidating...' : 'Run Dream'}
          </button>
        </div>

        {/* Frozen Markdown Prompt Docs + Reversible History */}
        <div className="z-surface-card flex flex-col gap-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <Tabs
              variant={tabsVariant}
              value={activeDoc}
              onValueChange={(v) => snap && pickDoc(v as MemoryDocName, snap)}
              className="min-w-0 max-w-full"
            >
              <TabsList>
                {MEMORY_DOC_NAMES.map((doc) => (
                  <TabsTrigger key={doc} value={doc} className="font-mono text-[11.5px]">
                    {doc}
                  </TabsTrigger>
                ))}
              </TabsList>
            </Tabs>
            <div className="flex items-center gap-1.5">
              <button
                type="button"
                className={showHistory ? 'z-btn z-btn-vermilion-soft z-btn-sm' : 'z-btn z-btn-ghost z-btn-sm'}
                onClick={() => setShowHistory((v) => !v)}
              >
                <History size={12.5} /> History ({activeDocHistory.length})
              </button>
              <button
                type="button"
                className="z-btn z-btn-primary z-btn-sm"
                onClick={async () => {
                  try {
                    await api(`/api/memory/doc/${encodeURIComponent(activeDoc)}`, {
                      method: 'PUT',
                      body: JSON.stringify({ content: docContent }),
                    })
                    docDirtyRef.current = false
                    toast(`Saved ${activeDoc} (snapshot archived)`, 'success')
                    load(query, scopeFilter, activeDoc, { forceSyncDoc: true })
                  } catch (e: any) {
                    toast(e?.message || `Failed to save ${activeDoc}`, 'error')
                  }
                }}
              >
                Save {activeDoc}
              </button>
            </div>
          </div>

          {showHistory && (
            <div className="flex flex-col gap-1.5 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] p-2.5">
              <div className="flex items-center justify-between text-[11.5px] text-[var(--muted)]">
                <span className="font-mono">.history/ snapshots for {activeDoc}</span>
                <span>1-click rollback saves current doc first</span>
              </div>
              {activeDocHistory.length === 0 ? (
                <p className="text-[12px] text-[var(--muted)]">No snapshots recorded for {activeDoc} yet.</p>
              ) : (
                activeDocHistory.slice(0, 8).map((s) => (
                  <div
                    key={s.id}
                    className="flex flex-wrap items-center justify-between gap-2 rounded border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1.5 text-[12px]"
                  >
                    <div className="min-w-0 font-mono text-[11px]">
                      <span>{s.id}</span>
                      <span className="ml-2 text-[var(--muted)]">[{s.reason}]</span>
                      <span className="ml-2 text-[var(--muted)]">{s.bytes} B</span>
                    </div>
                    <button
                      type="button"
                      className="z-btn z-btn-ghost z-btn-sm"
                      onClick={async () => {
                        try {
                          await api('/api/memory/rollback', {
                            method: 'POST',
                            body: JSON.stringify({ snapshot_id: s.id }),
                          })
                          toast(`Restored ${activeDoc} from ${s.id}`, 'success')
                          docDirtyRef.current = false
                          await load(query, scopeFilter, activeDoc, { forceSyncDoc: true })
                        } catch (e: any) {
                          toast(e.message || 'Rollback failed', 'error')
                        }
                      }}
                    >
                      <RotateCcw size={12} /> Restore
                    </button>
                  </div>
                ))
              )}
            </div>
          )}

          <textarea
            value={docContent}
            onChange={(e) => {
              docDirtyRef.current = true
              setDocContent(e.target.value)
            }}
            rows={10}
            className="w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] p-3 font-mono text-[12.5px] leading-relaxed text-[var(--fg)] focus:border-[var(--accent)] focus:outline-none"
          />
        </div>

        {/* Scoped Hybrid RRF Episodic Memory */}
        <div className="z-surface-card flex flex-col gap-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex flex-wrap items-center gap-2">
              <strong className="text-[14px]">Episodic Memory ({hits.length})</strong>
              <div className="flex items-center gap-1">
                {MEMORY_SCOPE_FILTERS.map((sf) => (
                  <button
                    key={sf.id}
                    type="button"
                    onClick={() => {
                      setScopeFilter(sf.id)
                      load(query, sf.id, activeDoc)
                    }}
                    className={
                      scopeFilter === sf.id
                        ? 'z-btn z-btn-vermilion-soft z-btn-sm'
                        : 'z-btn z-btn-ghost z-btn-sm'
                    }
                  >
                    {sf.label}
                  </button>
                ))}
              </div>
            </div>
            <div className="relative">
              <Search size={13} className="absolute left-2.5 top-2.5 text-[var(--muted)]" />
              <input
                value={query}
                onChange={(e) => {
                  const nextVal = e.target.value
                  setQuery(nextVal)
                  if (searchDebounceRef.current) window.clearTimeout(searchDebounceRef.current)
                  searchDebounceRef.current = window.setTimeout(() => {
                    load(nextVal, scopeFilter, activeDoc)
                  }, 250)
                }}
                placeholder="Hybrid RRF search..."
                className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] py-1.5 pl-8 pr-3 text-[12.5px] focus:border-[var(--accent)] focus:outline-none"
              />
            </div>
          </div>

          <div className="flex flex-wrap gap-2">
            <div className="w-28">
              <ZenkoSelect
                mono
                ariaLabel="Memory Scope"
                value={newScope}
                onChange={(v) => setNewScope(v as 'global' | 'project')}
                className="py-1.5"
                options={[
                  { value: 'global', label: 'global' },
                  { value: 'project', label: 'project' },
                ]}
              />
            </div>
            <div className="w-32">
              <ZenkoSelect
                mono
                ariaLabel="Memory Kind"
                value={newKind}
                onChange={setNewKind}
                className="py-1.5"
                options={memoryKinds.map((k) => ({
                  value: k.id,
                  label: k.id,
                  hint: k.description,
                }))}
              />
            </div>
            <input
              value={newTitle}
              onChange={(e) => setNewTitle(e.target.value)}
              placeholder="Title..."
              className="w-40 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-1.5 text-[12.5px]"
            />
            <input
              value={newContent}
              onChange={(e) => setNewContent(e.target.value)}
              placeholder="Fact, preference, or decision to remember..."
              className="min-w-[200px] flex-1 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-1.5 text-[12.5px]"
            />
            <button
              type="button"
              className="z-btn z-btn-primary z-btn-sm"
              onClick={async () => {
                if (!newContent.trim()) return
                await api('/api/memory/entries', {
                  method: 'POST',
                  body: JSON.stringify({
                    title: newTitle.trim() || 'Note',
                    content: newContent.trim(),
                    kind: newKind,
                    scope: newScope,
                  }),
                })
                setNewTitle('')
                setNewContent('')
                toast('Memory saved', 'success')
                load(query, scopeFilter, activeDoc)
              }}
            >
              <Plus size={13} /> Add
            </button>
          </div>

          <div className="flex flex-col gap-1.5">
            {hits.length === 0 ? (
              <p className="py-2 text-[12.5px] text-[var(--muted)]">No episodic memory entries matched this filter.</p>
            ) : (
              hits.map(({ entry: m, stale_paths }) => {
                const isEditing = editingId === m.id
                const scopeLabel = m.scope.startsWith('project:') ? 'project' : m.scope
                return (
                  <div
                    key={m.id}
                    className="flex items-start justify-between gap-2 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 text-[12.5px]"
                  >
                    {isEditing ? (
                      <div className="flex flex-1 flex-wrap items-center gap-2">
                        <input
                          value={editTitle}
                          onChange={(e) => setEditTitle(e.target.value)}
                          className="w-40 rounded border border-[var(--border)] bg-[var(--surface)] px-2 py-1 text-[12px]"
                        />
                        <input
                          value={editContent}
                          onChange={(e) => setEditContent(e.target.value)}
                          className="min-w-[180px] flex-1 rounded border border-[var(--border)] bg-[var(--surface)] px-2 py-1 text-[12px]"
                        />
                        <button
                          type="button"
                          className="z-btn z-btn-primary z-btn-sm"
                          onClick={async () => {
                            await api(`/api/memory/entries/${encodeURIComponent(m.id)}`, {
                              method: 'PATCH',
                              body: JSON.stringify({ title: editTitle, content: editContent }),
                            })
                            setEditingId(null)
                            toast('Memory updated', 'success')
                            load(query, scopeFilter, activeDoc)
                          }}
                        >
                          <Check size={12.5} />
                        </button>
                        <button
                          type="button"
                          className="z-btn z-btn-ghost z-btn-sm"
                          onClick={() => setEditingId(null)}
                        >
                          <X size={12.5} />
                        </button>
                      </div>
                    ) : (
                      <>
                        <div className="min-w-0 flex-1">
                          <div className="flex flex-wrap items-center gap-1.5">
                            <span className="rounded border border-[var(--border)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--muted)]">
                              {scopeLabel}
                            </span>
                            <span className="rounded border border-[var(--border)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--muted)]">
                              {m.kind}
                            </span>
                            <span className="font-medium">{m.title}</span>
                            {stale_paths.length > 0 && (
                              <span
                                title={`Missing paths: ${stale_paths.join(', ')}`}
                                className="inline-flex items-center gap-1 rounded border border-[var(--border)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--accent)]"
                              >
                                <AlertTriangle size={10.5} /> stale path
                              </span>
                            )}
                          </div>
                          <p className="mt-0.5 text-[var(--muted)]">{m.content}</p>
                        </div>
                        <div className="flex shrink-0 items-center gap-1.5 pt-0.5">
                          <button
                            type="button"
                            title="Edit memory"
                            onClick={() => {
                              setEditingId(m.id)
                              setEditTitle(m.title)
                              setEditContent(m.content)
                            }}
                            className="cursor-pointer text-[var(--muted)] hover:text-[var(--fg)]"
                          >
                            <Pencil size={13} />
                          </button>
                          <button
                            type="button"
                            title="Delete memory"
                            onClick={async () => {
                              await api(`/api/memory/entries/${encodeURIComponent(m.id)}`, { method: 'DELETE' })
                              load(query, scopeFilter, activeDoc)
                            }}
                            className="cursor-pointer text-[var(--muted)] hover:text-[var(--destructive)]"
                          >
                            <Trash2 size={13.5} />
                          </button>
                        </div>
                      </>
                    )}
                  </div>
                )
              })
            )}
          </div>
        </div>

        {/* Cross-Session FTS5 Transcript Recall (`session_search`) */}
        <div className="z-surface-card flex flex-col gap-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <strong className="text-[14px]">Past Sessions Recall (FTS5)</strong>
              <p className="text-[12px] text-[var(--muted)]">
                Sub-millisecond 3-tier BM25 search across historical transcripts, brainstorms & checkpoints.
              </p>
            </div>
            <div className="flex flex-wrap items-center gap-1">
              {SESSION_SINCE_FILTERS.map((f) => (
                <button
                  key={f.label}
                  type="button"
                  onClick={() => {
                    setSessionSince(f.id)
                    if (sessionQuery.trim()) searchSessions(sessionQuery, f.id, sessionWs)
                  }}
                  className={
                    sessionSince === f.id
                      ? 'z-btn z-btn-vermilion-soft z-btn-sm font-mono'
                      : 'z-btn z-btn-ghost z-btn-sm font-mono'
                  }
                >
                  {f.label}
                </button>
              ))}
              <button
                type="button"
                onClick={() => {
                  const next = sessionWs === 'current' ? 'all' : 'current'
                  setSessionWs(next)
                  if (sessionQuery.trim()) searchSessions(sessionQuery, sessionSince, next)
                }}
                className="z-btn z-btn-ghost z-btn-sm font-mono"
              >
                ws:{sessionWs}
              </button>
            </div>
          </div>

          <div className="flex gap-2">
            <div className="relative flex-1">
              <Search size={13} className="absolute left-2.5 top-2.5 text-[var(--muted)]" />
              <input
                value={sessionQuery}
                onChange={(e) => setSessionQuery(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') searchSessions()
                }}
                placeholder="Search past sessions by symbol, error, file, or topic..."
                className="w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] py-1.5 pl-8 pr-3 text-[12.5px] focus:border-[var(--accent)] focus:outline-none"
              />
            </div>
            <button
              type="button"
              disabled={sessionSearching}
              onClick={() => searchSessions()}
              className="z-btn z-btn-primary z-btn-sm"
            >
              {sessionSearching ? 'Searching...' : 'Search'}
            </button>
          </div>

          {sessionHits.length > 0 && (
            <div className="flex flex-col gap-1.5">
              {sessionHits.map((h) => (
                <div
                  key={h.event_id}
                  className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 text-[12.5px]"
                >
                  <div className="flex flex-wrap items-center justify-between gap-1.5">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <span className="font-medium">{h.session_title}</span>
                      <span className="rounded border border-[var(--border)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--muted)]">
                        {h.channel}
                      </span>
                      <span className="rounded border border-[var(--border)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--muted)]">
                        {h.role}
                      </span>
                      <span className="font-mono text-[10.5px] text-[var(--muted)]">
                        {h.session_id.slice(0, 12)}
                      </span>
                    </div>
                    <div className="flex items-center gap-1">
                      {(['jsonl', 'sharegpt', 'markdown'] as const).map((fmt) => (
                        <a
                          key={fmt}
                          href={`/api/sessions/${encodeURIComponent(h.session_id)}/export?format=${fmt}&anonymize=true`}
                          download
                          className="z-btn z-btn-ghost z-btn-sm font-mono text-[10px]"
                          title={`Download scrubbed ${fmt} trajectory`}
                        >
                          <Download size={10.5} /> {fmt}
                        </a>
                      ))}
                    </div>
                  </div>
                  <p className="mt-1 font-mono text-[11.5px] text-[var(--muted)]">{h.snippet}</p>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Phase 8 Procedural Skills, Draft Approval Gate & .history/ Rollback */}
        <div className="z-surface-card flex flex-col gap-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <div>
              <strong className="text-[14px]">Procedural Skills ({snap?.skills.length ?? 0})</strong>
              <p className="text-[12px] text-[var(--muted)]">
                Agent Skills standard (`~/.zenko/skills/`) with atomic `.history/` snapshots & draft approval gate.
              </p>
            </div>
            <button
              type="button"
              onClick={() => setShowSkillHistory((v) => !v)}
              className="z-btn z-btn-ghost z-btn-sm font-mono text-[11px]"
            >
              <History size={12} /> History ({skillHistory.length})
            </button>
          </div>

          <div className="grid gap-2 sm:grid-cols-3">
            <input
              value={newSkillSlug}
              onChange={(e) => setNewSkillSlug(e.target.value)}
              placeholder="slug (e.g. rust-perf-audit)"
              className="rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-2.5 py-1.5 font-mono text-[12px]"
            />
            <input
              value={newSkillBody}
              onChange={(e) => setNewSkillBody(e.target.value)}
              placeholder="Concise procedural steps (Markdown)..."
              className="sm:col-span-2 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-2.5 py-1.5 text-[12px]"
            />
          </div>
          <button
            type="button"
            className="z-btn z-btn-secondary z-btn-sm self-start"
            onClick={async () => {
              const slug = newSkillSlug.trim()
              const body = newSkillBody.trim()
              if (!slug || !body) {
                toast('Provide both skill slug and instructions', 'error')
                return
              }
              try {
                let safeSlugName = ''
                for (let i = 0; i < slug.length; i += 1) {
                  const ch = slug[i]
                  safeSlugName += ch === '\r' || ch === '\n' ? ' ' : ch === ':' ? '-' : ch
                }
                safeSlugName = safeSlugName.trim()
                const raw = `---\nname: ${safeSlugName}\ndescription: Reusable Zenko procedural skill\nenabled: true\nstatus: active\nauthor: user\n---\n\n${body}\n`
                await api(`/api/skills/${encodeURIComponent(slug)}`, {
                  method: 'PUT',
                  body: JSON.stringify({ content: raw }),
                })
                setNewSkillSlug('')
                setNewSkillBody('')
                toast(`Saved skill ${slug}`, 'success')
                load(query, scopeFilter, activeDoc)
              } catch (e: any) {
                toast(e.message, 'error')
              }
            }}
          >
            <Plus size={12.5} /> Save Skill
          </button>

          {showSkillHistory && skillHistory.length > 0 && (
            <div className="flex flex-col gap-1.5 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] p-2.5">
              <span className="font-mono text-[11px] text-[var(--muted)]">
                Recent skill `.history/` backups
              </span>
              {skillHistory.slice(0, 12).map((h) => (
                <div
                  key={h.id}
                  className="flex flex-wrap items-center justify-between gap-2 text-[11.5px]"
                >
                  <span className="font-mono text-[var(--muted)]">
                    {h.slug} · {h.reason} · {h.bytes}B
                  </span>
                  <button
                    type="button"
                    className="z-btn z-btn-ghost z-btn-sm font-mono text-[10.5px]"
                    onClick={async () => {
                      try {
                        await api('/api/skills/rollback', {
                          method: 'POST',
                          body: JSON.stringify({ snapshot_id: h.id }),
                        })
                        toast(`Rolled back skill ${h.slug}`, 'success')
                        load(query, scopeFilter, activeDoc)
                      } catch (e: any) {
                        toast(e.message, 'error')
                      }
                    }}
                  >
                    <RotateCcw size={11} /> Rollback
                  </button>
                </div>
              ))}
            </div>
          )}

          <div className="flex flex-col gap-1.5">
            {(snap?.skills ?? []).map((sk) => {
              const isDraft = sk.status === 'draft'
              return (
                <div
                  key={sk.slug}
                  className="flex flex-wrap items-center justify-between gap-2 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 text-[12.5px]"
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <span className="font-medium">{sk.name}</span>
                      <span className="font-mono text-[10.5px] text-[var(--muted)]">
                        {sk.slug}
                      </span>
                      <span
                        className="rounded border px-1.5 py-0.5 font-mono text-[10px] uppercase"
                        style={{
                          borderColor: isDraft ? 'var(--accent)' : 'var(--border)',
                          color: isDraft ? 'var(--accent)' : 'var(--muted)',
                        }}
                      >
                        {isDraft ? 'draft' : sk.enabled ? 'active' : 'disabled'}
                      </span>
                      {sk.author && sk.author !== 'user' && (
                        <span className="rounded border border-[var(--border)] px-1.5 py-0.5 font-mono text-[10px] text-[var(--muted)]">
                          {sk.author}
                        </span>
                      )}
                      <span className="font-mono text-[10.5px] text-[var(--muted)]">
                        ~{sk.token_estimate} tok
                      </span>
                    </div>
                    <p className="mt-0.5 truncate text-[11.5px] text-[var(--muted)]">
                      {sk.description}
                    </p>
                  </div>
                  <div className="flex shrink-0 items-center gap-1.5">
                    {isDraft && (
                      <button
                        type="button"
                        className="z-btn z-btn-vermilion-soft z-btn-sm font-mono text-[11px]"
                        onClick={async () => {
                          try {
                            await api(`/api/skills/${encodeURIComponent(sk.slug)}/approve`, {
                              method: 'POST',
                            })
                            toast(`Approved skill ${sk.slug}`, 'success')
                            load(query, scopeFilter, activeDoc)
                          } catch (e: any) {
                            toast(e.message, 'error')
                          }
                        }}
                      >
                        <Check size={12} /> Approve
                      </button>
                    )}
                    {!isDraft && (
                      <button
                        type="button"
                        className="z-btn z-btn-ghost z-btn-sm font-mono text-[11px]"
                        onClick={async () => {
                          try {
                            await api(`/api/skills/${encodeURIComponent(sk.slug)}/toggle`, {
                              method: 'POST',
                              body: JSON.stringify({ enabled: !sk.enabled }),
                            })
                            load(query, scopeFilter, activeDoc)
                          } catch (e: any) {
                            toast(e.message, 'error')
                          }
                        }}
                      >
                        {sk.enabled ? 'Disable' : 'Enable'}
                      </button>
                    )}
                    <button
                      type="button"
                      className="z-btn z-btn-danger z-btn-sm"
                      title="Delete skill"
                      onClick={async () => {
                        await api(`/api/skills/${encodeURIComponent(sk.slug)}`, {
                          method: 'DELETE',
                        })
                        toast(`Deleted skill ${sk.slug}`, 'info')
                        load(query, scopeFilter, activeDoc)
                      }}
                    >
                      <Trash2 size={12.5} />
                    </button>
                  </div>
                </div>
              )
            })}
          </div>
        </div>
      </div>
    </ScrollPage>
  )
}

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

// ─── Telemetry & Categorized Persistent Logs View ─────────────────────────────

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

// ─── Minimal Token Login Gate ─────────────────────────────────────────────────

export function LoginGate({ onLoggedIn }: { onLoggedIn: () => void }) {
  const [token, setToken] = useState('')
  const [error, setError] = useState('')

  return (
    <div className="flex h-full w-full items-center justify-center bg-[var(--bg)] p-6">
      <form
        onSubmit={async (e) => {
          e.preventDefault()
          setError('')
          try {
            await api('/api/login', { method: 'POST', body: JSON.stringify({ token }) })
            onLoggedIn()
          } catch (err: any) {
            setError(err.message)
          }
        }}
        className="z-surface-card flex w-full max-w-[360px] flex-col gap-4 p-6"
      >
        <div className="flex items-center gap-2.5">
          <ZenkoFoxLogo size={26} />
          <h1 className="text-[18px] font-semibold" style={{ fontFamily: 'var(--font-display)' }}>
            Zenko Control Plane
          </h1>
        </div>
        <p className="text-[12.5px] text-[var(--muted)]">
          Enter the admin token printed by <code className="font-mono">zenko setup</code> or{' '}
          <code className="font-mono">zenko web</code>.
        </p>
        {error && (
          <div className="rounded-lg border border-[var(--destructive)]/40 bg-[var(--destructive)]/10 px-3 py-1.5 text-[12px] text-[var(--destructive)]">
            {error}
          </div>
        )}
        <input
          type="password"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          placeholder="Admin token..."
          autoFocus
          required
          className="w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-2 font-mono text-[13px] focus:border-[var(--accent)] focus:outline-none"
        />
        <button type="submit" className="z-btn z-btn-primary w-full py-2.5">
          Unlock Zenko
        </button>
      </form>
    </div>
  )
}

function InputField({
  label,
  value,
  onChange,
  placeholder,
  type = 'text',
  mono = false,
}: {
  label: string
  value: string
  onChange: (v: string) => void
  placeholder?: string
  type?: string
  mono?: boolean
}) {
  return (
    <label className="flex flex-col gap-1 text-[11.5px] text-[var(--muted)]">
      <span>{label}</span>
      <input
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        className={`w-full rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--bg)] px-3 py-1.5 text-[13px] text-[var(--fg)] focus:border-[var(--accent)] focus:outline-none ${mono ? 'font-mono' : ''}`}
      />
    </label>
  )
}
