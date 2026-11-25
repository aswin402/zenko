import { useEffect, useRef, useState } from 'react'
import {
  Check,
  Flame,
  Globe,
  Layers,
  Palette,
  Plus,
  RefreshCw,
  Sliders,
  Trash2,
  X,
} from 'lucide-react'
import { api, formatCompactTokens } from '@/lib/api'
import type {
  ConciseMode,
  ConciseModeDescriptor,
  LspServerDescriptor,
  LspServerRuntimeStatus,
  McpServerStatus,
  ModelInfo,
  ProviderDescriptor,
  WebSearchProviderDescriptor,
} from '@/lib/types'
import { cn } from '@/lib/utils'
import {
  FALLBACK_CONCISE_MODES,
  type ConciseUpdatePatch,
} from '@/components/chat/CavemanModeControl'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { ZenkoSelect } from '@/components/ui/ZenkoSelect'
import { Tabs, TabsList, TabsTrigger } from '@/components/motion/tabs'
import { InputField, ScrollPage, splitShellArgs, type SecondaryTabsVariant } from './shared'

// ─── Setup & Settings View ────────────────────────────────────────────────────

export function SetupView({
  config,
  providers,
  configuredProviders,
  conciseModes = [],
  conciseMode = 'lite',
  autoClarityEnabled = true,
  readLessEnabled = true,
  onUpdateConcise,
  onCustomizeConciseButton,
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
  conciseModes?: ConciseModeDescriptor[]
  conciseMode?: ConciseMode
  autoClarityEnabled?: boolean
  readLessEnabled?: boolean
  onUpdateConcise?: (patch: ConciseUpdatePatch) => Promise<void> | void
  onCustomizeConciseButton?: () => void
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
  const [setupSection, setSetupSection] = useState<'all' | 'core' | 'extensions'>('all')
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
  const [savingConcise, setSavingConcise] = useState(false)
  const lastActiveConciseRef = useRef<ConciseMode>(conciseMode !== 'off' ? conciseMode : 'lite')

  useEffect(() => {
    if (conciseMode !== 'off') {
      lastActiveConciseRef.current = conciseMode
    }
  }, [conciseMode])

  const effectiveConciseModes =
    conciseModes.length > 0 ? conciseModes : FALLBACK_CONCISE_MODES
  const cavemanOn = conciseMode !== 'off'

  const applyConcisePatch = async (patch: ConciseUpdatePatch) => {
    if (!onUpdateConcise) return
    setSavingConcise(true)
    try {
      await onUpdateConcise(patch)
    } finally {
      setSavingConcise(false)
    }
  }

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
          concise_mode: conciseMode,
          auto_clarity: autoClarityEnabled,
          read_less: readLessEnabled,
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
        <div className="flex flex-wrap items-end justify-between gap-3">
          <div>
            <h1 className="text-[22px] font-semibold tracking-[-0.03em]" style={{ fontFamily: 'var(--font-display)' }}>
              Settings & Setup
            </h1>
            <p className="mt-1 text-[13px] text-[var(--muted)]">
              Secrets are encrypted at rest with ChaCha20-Poly1305 and never exposed to shell tools or LLM context.
            </p>
          </div>
          <Tabs
            value={setupSection}
            onValueChange={(v) => setSetupSection(v as 'all' | 'core' | 'extensions')}
            variant={tabsVariant}
          >
            <TabsList>
              <TabsTrigger value="all" icon={<Layers size={13} />}>All</TabsTrigger>
              <TabsTrigger value="core" icon={<Sliders size={13} />}>Providers &amp; Policy</TabsTrigger>
              <TabsTrigger value="extensions" icon={<Globe size={13} />}>Search &amp; MCP</TabsTrigger>
            </TabsList>
          </Tabs>
        </div>

        {setupSection !== 'extensions' && (
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

          {/* Caveman Mode & Output Efficiency (Live Synced with Chat Input & Runtime) */}
          <div className="z-surface-card flex flex-col gap-3.5 md:col-span-2">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div className="flex items-start gap-2.5">
                <div
                  className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border"
                  style={
                    cavemanOn
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
                  <Flame size={15} />
                </div>
                <div>
                  <div className="flex items-center gap-2">
                    <strong className="text-[14px]">Caveman Mode &amp; Output Efficiency</strong>
                    <span
                      className="rounded-full border px-2 py-0.5 font-mono text-[10.5px]"
                      style={
                        cavemanOn
                          ? {
                              color: 'var(--accent)',
                              borderColor: 'color-mix(in srgb, var(--accent) 42%, transparent)',
                              backgroundColor: 'color-mix(in srgb, var(--accent) 10%, transparent)',
                            }
                          : {
                              color: 'var(--muted)',
                              borderColor: 'var(--border)',
                              backgroundColor: 'var(--surface)',
                            }
                      }
                    >
                      {cavemanOn ? `ON · ${conciseMode.toUpperCase()}` : 'OFF'}
                    </span>
                  </div>
                  <p className="mt-0.5 text-[12px] text-[var(--muted)]">
                    Pure-English token compression synced live with the chat input bar and persisted to{' '}
                    <code className="font-mono">zenko.toml</code>. Code blocks, paths, and errors stay verbatim.
                  </p>
                </div>
              </div>

              <div className="flex items-center gap-2.5">
                {onCustomizeConciseButton && (
                  <button
                    type="button"
                    onClick={onCustomizeConciseButton}
                    className="z-btn z-btn-secondary z-btn-sm"
                  >
                    <Palette size={12} />
                    <span>Customize Chat Button</span>
                  </button>
                )}
                <button
                  type="button"
                  role="switch"
                  aria-label="Toggle Caveman Mode"
                  aria-checked={cavemanOn}
                  disabled={savingConcise}
                  onClick={() =>
                    applyConcisePatch({
                      concise_mode: cavemanOn ? 'off' : lastActiveConciseRef.current,
                    })
                  }
                  className="relative inline-flex h-6 w-11 shrink-0 cursor-pointer items-center rounded-full border transition-colors disabled:opacity-50"
                  style={{
                    backgroundColor: cavemanOn ? 'var(--accent)' : 'var(--bg)',
                    borderColor: cavemanOn ? 'var(--accent)' : 'var(--border)',
                  }}
                >
                  <span
                    className={cn(
                      'inline-block h-4 w-4 rounded-full bg-white shadow-xs transition-transform',
                      cavemanOn ? 'translate-x-5.5' : 'translate-x-1',
                    )}
                  />
                </button>
              </div>
            </div>

            <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-4">
              {effectiveConciseModes.map((m) => {
                const selected = conciseMode === m.id
                return (
                  <button
                    key={m.id}
                    type="button"
                    disabled={savingConcise}
                    onClick={() => applyConcisePatch({ concise_mode: m.id })}
                    className={cn(
                      'flex cursor-pointer flex-col gap-1 rounded-xl border p-3 text-left transition-all disabled:opacity-60',
                      selected
                        ? 'border-[var(--accent)] bg-[var(--accent)]/10 text-[var(--fg)]'
                        : 'border-[var(--border)] bg-[var(--bg)]/60 text-[var(--fg)]/85 hover:border-[var(--muted)]/55',
                    )}
                  >
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-[12.5px] font-semibold">{m.label}</span>
                      <div className="flex items-center gap-1.5">
                        <span className="font-mono text-[10px] text-[var(--muted)]">
                          {m.target_reduction_pct > 0 ? `-${m.target_reduction_pct}% tok` : 'Default'}
                        </span>
                        {selected && (
                          <span
                            className="flex h-4 w-4 items-center justify-center rounded-full text-[var(--primary-fg)]"
                            style={{ backgroundColor: 'var(--accent)' }}
                          >
                            <Check size={10} strokeWidth={2.8} />
                          </span>
                        )}
                      </div>
                    </div>
                    <span className="text-[11px] leading-snug text-[var(--muted)]">
                      {m.description}
                    </span>
                  </button>
                )
              })}
            </div>

            <div className="grid grid-cols-1 gap-3 border-t border-[var(--border)]/70 pt-3 sm:grid-cols-2">
              <div className="flex items-center justify-between gap-3 rounded-xl border border-[var(--border)] bg-[var(--bg)]/50 px-3.5 py-2.5">
                <div>
                  <div className="text-[12.5px] font-medium text-[var(--fg)]">
                    Auto-Clarity Safety Hatch
                  </div>
                  <div className="text-[11px] text-[var(--muted)]">
                    Automatically expands to full clear English for security warnings or irreversible ops.
                  </div>
                </div>
                <button
                  type="button"
                  role="switch"
                  aria-label="Toggle Auto-Clarity"
                  aria-checked={autoClarityEnabled}
                  disabled={savingConcise}
                  onClick={() => applyConcisePatch({ auto_clarity: !autoClarityEnabled })}
                  className="relative inline-flex h-5.5 w-10 shrink-0 cursor-pointer items-center rounded-full border transition-colors"
                  style={{
                    backgroundColor: autoClarityEnabled ? 'var(--accent)' : 'var(--bg)',
                    borderColor: autoClarityEnabled ? 'var(--accent)' : 'var(--border)',
                  }}
                >
                  <span
                    className={cn(
                      'inline-block h-3.5 w-3.5 rounded-full bg-white shadow-xs transition-transform',
                      autoClarityEnabled ? 'translate-x-5' : 'translate-x-1',
                    )}
                  />
                </button>
              </div>

              <div className="flex items-center justify-between gap-3 rounded-xl border border-[var(--border)] bg-[var(--bg)]/50 px-3.5 py-2.5">
                <div>
                  <div className="text-[12.5px] font-medium text-[var(--fg)]">
                    Read-Less Tool Compression
                  </div>
                  <div className="text-[11px] text-[var(--muted)]">
                    Compresses verbose JSON, CSV, test logs, and build output before LLM context (never alters <code className="font-mono">read_file</code>).
                  </div>
                </div>
                <button
                  type="button"
                  role="switch"
                  aria-label="Toggle Read-Less Tool Compression"
                  aria-checked={readLessEnabled}
                  disabled={savingConcise}
                  onClick={() => applyConcisePatch({ read_less: !readLessEnabled })}
                  className="relative inline-flex h-5.5 w-10 shrink-0 cursor-pointer items-center rounded-full border transition-colors"
                  style={{
                    backgroundColor: readLessEnabled ? 'var(--accent)' : 'var(--bg)',
                    borderColor: readLessEnabled ? 'var(--accent)' : 'var(--border)',
                  }}
                >
                  <span
                    className={cn(
                      'inline-block h-3.5 w-3.5 rounded-full bg-white shadow-xs transition-transform',
                      readLessEnabled ? 'translate-x-5' : 'translate-x-1',
                    )}
                  />
                </button>
              </div>
            </div>
          </div>
        </div>
        )}

        {/* Phase 7: Pluggable Web Search, External MCP Client Servers & Pooled LSP */}
        {setupSection !== 'core' && (
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
        )}
      </div>
    </ScrollPage>
  )
}

