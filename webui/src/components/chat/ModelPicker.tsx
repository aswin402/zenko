import { useEffect, useMemo, useRef, useState } from 'react'
import {
  ArrowLeft,
  Check,
  ChevronDown,
  ChevronRight,
  Cpu,
  KeyRound,
  RefreshCw,
  Search,
} from 'lucide-react'
import { AnimatePresence, motion } from 'motion/react'
import { api, formatCompactTokens } from '@/lib/api'
import { SPRING_PANEL } from '@/lib/ease'
import type { ModelInfo, ProviderDescriptor } from '@/lib/types'
import { cn } from '@/lib/utils'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { ZenkoTooltip } from '@/components/ui/Tooltip'
import { ModelPicker as RareModelPicker, type PickerModel } from '@/components/rareui/model-picker'

function formatModelPrice(m: ModelInfo): string {
  if (m.is_free) return 'FREE'
  if (m.prompt_price_per_m == null) return ''
  return `$${m.prompt_price_per_m.toFixed(m.prompt_price_per_m < 1 ? 2 : 1)}/M`
}

interface ModelPickerProps {
  activeProvider: string
  activeModel: string
  providers: ProviderDescriptor[]
  configuredProviders: Record<string, boolean>
  activeModels: ModelInfo[]
  pickerStyle: 'drilldown' | 'split_pane' | 'compact_inline' | 'rareui_picker'
  pillClass: string
  open: boolean
  onOpenChange: (next: boolean) => void
  onSelectProviderModel: (providerId: string, modelId: string) => void
  onOpenSettings?: () => void
  inlinePreview?: boolean
}

/**
 * Minimal 2-level Provider -> Live Models picker (zero hardcoded providers or models).
 * Click trigger -> view Providers -> click Provider -> pick live working model (FREE first).
 */
export function ModelPicker({
  activeProvider,
  activeModel,
  providers,
  configuredProviders,
  activeModels,
  pickerStyle,
  pillClass,
  open,
  onOpenChange,
  onSelectProviderModel,
  onOpenSettings,
  inlinePreview = false,
}: ModelPickerProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const [step, setStep] = useState<'providers' | 'models'>('providers')
  const [browsingProvider, setBrowsingProvider] = useState<string>(activeProvider)
  const [cache, setCache] = useState<Record<string, ModelInfo[]>>({})
  const [loading, setLoading] = useState(false)
  const [query, setQuery] = useState('')
  const [freeOnly, setFreeOnly] = useState(false)
  const reqSeq = useRef(0)

  // Seed cache with active provider models already bootstrapped
  useEffect(() => {
    if (activeProvider && activeModels.length > 0) {
      setCache((prev) => ({ ...prev, [activeProvider]: activeModels }))
    }
  }, [activeProvider, activeModels])

  // Close popover on outside click or Escape key (non-preview mode)
  useEffect(() => {
    if (!open || inlinePreview) return
    const onPointer = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        onOpenChange(false)
      }
    }
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        onOpenChange(false)
      }
    }
    window.addEventListener('mousedown', onPointer)
    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('mousedown', onPointer)
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [open, inlinePreview, onOpenChange])

  // Reset to Providers step every time drilldown popover opens
  useEffect(() => {
    if (open) {
      setQuery('')
      setFreeOnly(false)
      setBrowsingProvider(activeProvider || providers[0]?.id || '')
      setStep(pickerStyle === 'drilldown' ? 'providers' : 'models')
    }
  }, [open, activeProvider, pickerStyle, providers])

  const fetchProviderModels = async (providerId: string, refresh = false) => {
    if (!providerId) return
    if (!refresh && cache[providerId] && cache[providerId].length > 0) return
    const seq = ++reqSeq.current
    setLoading(true)
    try {
      const qs = new URLSearchParams({ provider: providerId })
      if (refresh) qs.set('refresh', 'true')
      const list = await api<ModelInfo[]>(`/api/models?${qs.toString()}`)
      if (seq !== reqSeq.current) return
      setCache((prev) => ({ ...prev, [providerId]: list }))
    } catch {
      if (seq === reqSeq.current) {
        setCache((prev) => ({ ...prev, [providerId]: prev[providerId] || [] }))
      }
    } finally {
      if (seq === reqSeq.current) setLoading(false)
    }
  }

  // Auto-load models when browsing in split_pane or compact_inline
  useEffect(() => {
    if ((open || inlinePreview) && pickerStyle !== 'drilldown' && browsingProvider) {
      fetchProviderModels(browsingProvider)
    }
  }, [open, inlinePreview, pickerStyle, browsingProvider])

  const currentDesc = providers.find((p) => p.id === browsingProvider) || providers[0]
  const activeDesc = providers.find((p) => p.id === activeProvider)
  const browsingModels = cache[browsingProvider] || (browsingProvider === activeProvider ? activeModels : [])
  const activeMeta = activeModels.find((m) => m.id === activeModel)
  const shortModel = activeModel
    ? activeModel.includes('/')
      ? activeModel.split('/').pop()
      : activeModel
    : 'Select model'

  const freeCount = useMemo(
    () => browsingModels.filter((m) => m.is_free).length,
    [browsingModels],
  )

  const filteredModels = useMemo(() => {
    const q = query.trim().toLowerCase()
    return browsingModels.filter((m) => {
      if (freeOnly && !m.is_free) return false
      if (!q) return true
      return m.id.toLowerCase().includes(q) || m.name.toLowerCase().includes(q)
    })
  }, [browsingModels, query, freeOnly])

  const enterProvider = (providerId: string) => {
    setBrowsingProvider(providerId)
    setQuery('')
    setFreeOnly(false)
    setStep('models')
    fetchProviderModels(providerId)
  }

  const pickModel = (providerId: string, modelId: string) => {
    onSelectProviderModel(providerId, modelId)
    setQuery('')
    if (!inlinePreview) onOpenChange(false)
  }

  const renderProviderList = (compact = false) => (
    <div className="flex flex-col gap-0.5">
      {!compact && (
        <div className="mb-1 flex items-center justify-between px-2 pt-0.5">
          <span className="text-[10.5px] font-semibold tracking-wider text-[var(--muted)] uppercase">
            Select Provider
          </span>
          <span className="font-mono text-[10px] text-[var(--muted)]">
            {providers.length} available
          </span>
        </div>
      )}
      {providers.map((p) => {
        const isCurrent = p.id === activeProvider
        const isBrowsing = p.id === browsingProvider
        const isReady = !p.requires_api_key || Boolean(configuredProviders[p.id])
        return (
          <button
            key={p.id}
            type="button"
            onClick={() => enterProvider(p.id)}
            className={cn(
              'group flex w-full cursor-pointer items-center justify-between gap-2 rounded-lg px-2.5 py-2 text-left transition-colors',
              compact && isBrowsing
                ? 'bg-[var(--card)] text-[var(--fg)]'
                : 'text-[var(--fg)]/88 hover:bg-[var(--card)]',
            )}
          >
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-1.5">
                <span className="truncate text-[12.5px] font-medium">{p.label}</span>
                {isCurrent && (
                  <span
                    className="rounded px-1.5 py-0.5 font-mono text-[9px] font-semibold uppercase"
                    style={{
                      color: 'var(--accent)',
                      backgroundColor: 'color-mix(in srgb, var(--accent) 14%, transparent)',
                    }}
                  >
                    active
                  </span>
                )}
              </div>
              {!compact && (
                <div className="truncate text-[10.5px] text-[var(--muted)]">
                  {!p.requires_api_key
                    ? 'Local · Free'
                    : isReady
                      ? 'Key configured'
                      : 'Configure key in Settings'}
                </div>
              )}
            </div>
            <ChevronRight
              size={13}
              className="shrink-0 text-[var(--muted)] transition-transform group-hover:translate-x-0.5 group-hover:text-[var(--fg)]"
            />
          </button>
        )
      })}
    </div>
  )

  const renderModelsPane = (showBack: boolean) => {
    const needsKey = currentDesc?.requires_api_key && !configuredProviders[browsingProvider]
    return (
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="mb-2 flex items-center justify-between gap-2 border-b border-[var(--border)] pb-2">
          <div className="flex min-w-0 items-center gap-1.5">
            {showBack && (
              <button
                type="button"
                onClick={() => setStep('providers')}
                className="inline-flex cursor-pointer items-center gap-1 rounded-md px-1.5 py-1 text-[11px] font-medium text-[var(--muted)] transition-colors hover:bg-[var(--card)] hover:text-[var(--fg)]"
              >
                <ArrowLeft size={12} />
                <span>Providers</span>
              </button>
            )}
            <span className="truncate text-[12.5px] font-semibold text-[var(--fg)]">
              {currentDesc?.label || browsingProvider}
            </span>
          </div>
          <ZenkoTooltip label="Refresh models" side="top">
            <button
              type="button"
              disabled={loading}
              onClick={() => fetchProviderModels(browsingProvider, true)}
              className="flex h-6 w-6 shrink-0 cursor-pointer items-center justify-center rounded-md text-[var(--muted)] transition-colors hover:bg-[var(--card)] hover:text-[var(--fg)]"
            >
              <RefreshCw size={11.5} className={loading ? 'animate-spin' : ''} />
            </button>
          </ZenkoTooltip>
        </div>

        <div className="relative mb-2">
          <Search
            size={12}
            className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-[var(--muted)]"
          />
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && query.trim()) {
                const target = filteredModels[0]?.id || query.trim()
                pickModel(browsingProvider, target)
              }
            }}
            placeholder="Search models or paste ID..."
            className="w-full rounded-lg border border-[var(--border)] bg-[var(--bg)] py-1.5 pl-7 pr-2.5 font-mono text-[11.5px] text-[var(--fg)] placeholder:text-[var(--muted)]/60 focus:border-[var(--accent)] focus:outline-none"
          />
        </div>

        <div className="mb-1.5 flex items-center justify-between text-[10.5px]">
          <div className="flex items-center gap-1">
            <button
              type="button"
              onClick={() => setFreeOnly(false)}
              className={cn(
                'cursor-pointer rounded-md px-2 py-0.5 font-mono text-[10.5px] transition-colors',
                !freeOnly
                  ? 'bg-[var(--card)] font-medium text-[var(--fg)]'
                  : 'text-[var(--muted)] hover:text-[var(--fg)]',
              )}
            >
              All ({browsingModels.length})
            </button>
            <button
              type="button"
              onClick={() => setFreeOnly(true)}
              className={cn(
                'cursor-pointer rounded-md px-2 py-0.5 font-mono text-[10.5px] transition-colors',
                freeOnly
                  ? 'font-medium text-[var(--accent)]'
                  : 'text-[var(--muted)] hover:text-[var(--fg)]',
              )}
              style={
                freeOnly
                  ? { backgroundColor: 'color-mix(in srgb, var(--accent) 13%, transparent)' }
                  : undefined
              }
            >
              Free ({freeCount})
            </button>
          </div>
          {needsKey && onOpenSettings && (
            <button
              type="button"
              onClick={() => {
                onOpenChange(false)
                onOpenSettings()
              }}
              className="inline-flex cursor-pointer items-center gap-1 font-mono text-[10px] text-[var(--accent)] hover:underline"
            >
              <KeyRound size={10} /> Add Key
            </button>
          )}
        </div>

        <CompactScrollArea autoHeight viewportClassName="max-h-60 pr-2.5">
          {loading ? (
            <div className="py-6 text-center font-mono text-[11px] text-[var(--muted)]">
              Loading live models...
            </div>
          ) : filteredModels.length === 0 ? (
            <div className="py-5 text-center font-mono text-[11px] text-[var(--muted)]">
              {query.trim()
                ? 'Press Enter to use this custom model ID'
                : 'No live models available — click Refresh or add API key'}
            </div>
          ) : (
            filteredModels.slice(0, 60).map((m) => {
              const selected = browsingProvider === activeProvider && m.id === activeModel
              return (
                <button
                  key={m.id}
                  type="button"
                  onClick={() => pickModel(browsingProvider, m.id)}
                  className={cn(
                    'flex w-full cursor-pointer items-center justify-between gap-2 rounded-lg px-2 py-1.5 text-left transition-colors',
                    selected
                      ? 'bg-[var(--card)] text-[var(--fg)]'
                      : 'hover:bg-[var(--card)]/70',
                  )}
                >
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-1.5">
                      {selected && (
                        <Check size={11.5} className="shrink-0" style={{ color: 'var(--accent)' }} />
                      )}
                      <span className="truncate text-[12px] font-medium text-[var(--fg)]">
                        {m.name}
                      </span>
                      {m.is_free && (
                        <span
                          className="shrink-0 rounded px-1 font-mono text-[9.5px] font-semibold"
                          style={{
                            color: 'var(--accent)',
                            backgroundColor: 'color-mix(in srgb, var(--accent) 14%, transparent)',
                          }}
                        >
                          FREE
                        </span>
                      )}
                    </div>
                    <div className="truncate font-mono text-[10px] text-[var(--muted)]">{m.id}</div>
                  </div>
                  <div className="flex shrink-0 flex-col items-end font-mono text-[10px] text-[var(--muted)]">
                    <span>
                      {m.context_length != null ? formatCompactTokens(m.context_length) : '—'}
                    </span>
                    {!m.is_free && m.prompt_price_per_m != null && (
                      <span className="text-[9.5px] opacity-75">{formatModelPrice(m)}</span>
                    )}
                  </div>
                </button>
              )
            })
          )}
        </CompactScrollArea>
      </div>
    )
  }

  const rareModels = useMemo<PickerModel[]>(() => {
    const providerLabel = activeDesc?.label || activeProvider
    const list = activeModels.slice(0, 60).map((m) => {
      const ctxLabel = m.context_length != null ? formatCompactTokens(m.context_length) : 'live'
      const priceLabel = formatModelPrice(m)
      return {
        id: m.id,
        name: m.name || m.id,
        provider: providerLabel,
        icon: <Cpu size={13} style={{ color: 'var(--accent)' }} />,
        description: `${m.id} · ${ctxLabel}${priceLabel ? ` · ${priceLabel}` : ''}`,
        speed: m.is_free ? 4 : 5,
        intelligence: m.supports_reasoning ? 5 : 4,
      }
    })
    if (activeModel && !list.some((m) => m.id === activeModel)) {
      list.unshift({
        id: activeModel,
        name: shortModel || activeModel,
        provider: providerLabel,
        icon: <Cpu size={13} style={{ color: 'var(--accent)' }} />,
        description: activeModel,
        speed: 4,
        intelligence: 5,
      })
    }
    return list
  }, [activeModels, activeModel, activeProvider, activeDesc?.label, shortModel])

  if (pickerStyle === 'rareui_picker' && rareModels.length > 0) {
    return (
      <div className="inline-flex items-center gap-1">
        <RareModelPicker
          models={rareModels}
          value={activeModel || rareModels[0]?.id}
          onChange={(nextModelId) => onSelectProviderModel(activeProvider, nextModelId)}
          searchable
          className="rounded-full border border-[var(--border)] bg-[var(--card)] text-[var(--fg)]"
        />
      </div>
    )
  }

  const popoverBody = (
    <div
      className={cn(
        'rounded-2xl border border-[var(--border)] bg-[var(--surface)] p-2.5 shadow-[0_18px_48px_rgba(0,0,0,0.45)]',
        pickerStyle === 'split_pane' ? 'w-[470px] max-w-[90vw]' : 'w-[330px] max-w-[90vw]',
      )}
    >
      {pickerStyle === 'split_pane' ? (
        <div className="flex gap-2.5">
          <div className="w-[152px] shrink-0 border-r border-[var(--border)] pr-2">
            <div className="mb-1.5 px-2 text-[10px] font-semibold tracking-wider text-[var(--muted)] uppercase">
              Providers
            </div>
            {renderProviderList(true)}
          </div>
          {renderModelsPane(false)}
        </div>
      ) : pickerStyle === 'compact_inline' ? (
        <div className="flex flex-col gap-2">
          <div className="flex flex-wrap gap-1 border-b border-[var(--border)] pb-2">
            {providers.map((p) => {
              const active = p.id === browsingProvider
              return (
                <button
                  key={p.id}
                  type="button"
                  onClick={() => enterProvider(p.id)}
                  className={cn(
                    'cursor-pointer rounded-full border px-2 py-0.5 text-[11px] font-medium transition-colors',
                    active
                      ? 'border-[var(--accent)]/50 text-[var(--fg)]'
                      : 'border-[var(--border)] text-[var(--muted)] hover:text-[var(--fg)]',
                  )}
                  style={
                    active
                      ? { backgroundColor: 'color-mix(in srgb, var(--accent) 14%, transparent)' }
                      : undefined
                  }
                >
                  {p.label}
                </button>
              )
            })}
          </div>
          {renderModelsPane(false)}
        </div>
      ) : step === 'providers' ? (
        renderProviderList(false)
      ) : (
        renderModelsPane(true)
      )}
    </div>
  )

  if (inlinePreview) {
    return <div className="inline-block">{popoverBody}</div>
  }

  return (
    <div ref={containerRef} className="relative">
      <button
        type="button"
        onClick={() => onOpenChange(!open)}
        className={cn(
          'inline-flex h-7 cursor-pointer items-center gap-1.5 rounded-full px-2.5 text-[12px] transition-colors',
          pillClass,
        )}
      >
        <Cpu size={12.5} style={{ color: 'var(--accent)' }} />
        {activeDesc && (
          <span className="hidden text-[11px] text-[var(--muted)] sm:inline">
            {activeDesc.label}
          </span>
        )}
        {activeDesc && <span className="hidden text-[var(--border)] sm:inline">/</span>}
        <span className="max-w-[155px] truncate font-medium text-[var(--fg)]">{shortModel}</span>
        {activeMeta?.is_free && (
          <span
            className="rounded px-1 font-mono text-[9.5px] font-semibold"
            style={{
              color: 'var(--accent)',
              backgroundColor: 'color-mix(in srgb, var(--accent) 14%, transparent)',
            }}
          >
            FREE
          </span>
        )}
        <ChevronDown
          size={12}
          className={cn('text-[var(--muted)] transition-transform', open && 'rotate-180')}
        />
      </button>

      <AnimatePresence>
        {open && (
          <motion.div
            initial={{ opacity: 0, y: 6, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 6, scale: 0.98 }}
            transition={SPRING_PANEL}
            className="absolute bottom-9 left-0 z-50"
          >
            {popoverBody}
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  )
}
