import { memo, useEffect, useRef, useState, type ReactNode } from 'react'
import {
  ArrowUp,
  CornerDownLeft,
  RefreshCw,
  Send,
  Sparkles,
  X,
  type LucideIcon,
} from 'lucide-react'
import { motion } from 'motion/react'
import { SPRING_PRESS } from '@/lib/ease'
import { api, formatCompactTokens } from '@/lib/api'
import type {
  ComponentSlots,
  ContextBreakdown,
  ModelInfo,
  PermissionMode,
  ProviderDescriptor,
  QueuedPrompt,
} from '@/lib/types'
import { cn } from '@/lib/utils'
import { ProgressRing } from '@/components/beui/ProgressRing'
import { ZenkoIcon } from '@/components/beui/ZenkoIcon'
import { ModelPicker } from '@/components/chat/ModelPicker'
import { KokonutContextMeter } from '@/components/kokonutui/ai-context-meter'
import { PromptInput as MotionBrewPromptInput } from '@/components/motion-brew/prompt-input'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/motion/popover'
import { SmoothCaretTextarea } from '@/components/skiperui/smooth-caret'
import AIContextMeter, {
  CONTEXT_METER_CHROME_CLASS,
  type ContextMeterChrome,
} from '@/components/smoothui/ai-context-meter'
import { ComposerBeam } from '@/components/ui/ComposerBeam'
import { ZenkoTooltip } from '@/components/ui/Tooltip'

const SHELL_VARIANT_CLASS: Record<ComponentSlots['composer']['shell_variant'], string> = {
  floating_card:
    'rounded-[20px] border border-[var(--border)] bg-[var(--surface)]/96 px-4 pb-3 pt-3.5 shadow-[0_12px_36px_rgba(0,0,0,0.38),inset_0_1px_0_rgba(255,255,255,0.04)] focus-within:border-[var(--accent)]/55 focus-within:shadow-[0_12px_36px_rgba(0,0,0,0.42),0_0_0_1px_color-mix(in_srgb,var(--accent)_22%,transparent)]',
  pill_capsule:
    'rounded-[28px] border border-[var(--border)] bg-[var(--card)]/95 px-5 pb-3 pt-3.5 shadow-[0_10px_30px_rgba(0,0,0,0.34)] focus-within:border-[var(--accent)]/60',
  minimal_bar:
    'rounded-xl border border-[var(--border)] bg-[var(--surface)] px-3.5 pb-2.5 pt-3 focus-within:border-[var(--fg)]/30',
  glass_dock:
    'rounded-[20px] border border-[var(--border)] bg-[var(--card)]/74 px-4 pb-3 pt-3.5 backdrop-blur-md shadow-[0_16px_42px_rgba(0,0,0,0.36)] focus-within:border-[var(--accent)]/55',
  motion_brew_prompt: '',
}

const COMPACT_SEGMENTS = 12
function CompactSegmentedRing({ ratio }: { ratio: number }) {
  const lit = Math.round((Math.min(100, Math.max(0, ratio)) / 100) * COMPACT_SEGMENTS)
  const r = 6.5
  const cx = 9
  const cy = 9
  const step = 360 / COMPACT_SEGMENTS
  return (
    <svg viewBox="0 0 18 18" className="size-4 -rotate-90" aria-hidden="true">
      {Array.from({ length: COMPACT_SEGMENTS }, (_, i) => {
        const a0 = ((i * step + 6) * Math.PI) / 180
        const a1 = (((i + 1) * step - 6) * Math.PI) / 180
        const d = `M ${cx + r * Math.cos(a0)} ${cy + r * Math.sin(a0)} A ${r} ${r} 0 0 1 ${cx + r * Math.cos(a1)} ${cy + r * Math.sin(a1)}`
        return (
          <path
            key={i}
            d={d}
            fill="none"
            stroke={i < lit ? 'var(--accent)' : 'currentColor'}
            strokeOpacity={i < lit ? 1 : 0.22}
            strokeWidth={2}
            strokeLinecap="round"
          />
        )
      })}
    </svg>
  )
}

const ACTION_PILL_CLASS: Record<ComponentSlots['composer']['action_pills'], string> = {
  soft_tint: 'border border-[var(--border)] bg-[var(--card)] text-[var(--fg)]/90 hover:border-[var(--muted)]/55',
  outline: 'border border-[var(--border)] bg-transparent text-[var(--fg)]/88 hover:border-[var(--fg)]/35',
  ghost: 'border border-transparent bg-transparent text-[var(--muted)] hover:bg-[var(--card)] hover:text-[var(--fg)]',
  solid_chip: 'border border-[var(--border)]/60 bg-[var(--bg)] text-[var(--fg)] shadow-xs hover:border-[var(--accent)]/45',
}

const SEND_ICON_MAP: Record<ComponentSlots['composer']['send_button_icon'], LucideIcon> = {
  arrow_up: ArrowUp,
  send: Send,
  sparkles: Sparkles,
  corner_up: CornerDownLeft,
}

const SEND_SHAPE_CLASS: Record<ComponentSlots['composer']['send_button_shape'], string> = {
  rounded_square: 'z-btn-icon h-8 w-8 rounded-[10px]',
  circle: 'z-btn-icon h-8 w-8 rounded-full',
  pill_label: 'h-8 rounded-full px-3 text-[12px] font-medium',
}

const POLICY_OPTIONS = [
  ['autonomous', 'Autonomous', 'Full freedom · ask only on dangerous shell'],
  ['balanced', 'Balanced', 'Auto-run edits & safe tools · ask other shell'],
  ['strict', 'Strict', 'Ask before writes & shell'],
  ['plan', 'Plan', 'Read-only explore & plan'],
] as const

type ComposerDispatchState = 'idle' | 'running_foreground' | 'running_orchestrating'

const COMPOSER_STATE_META: Record<
  ComposerDispatchState,
  { placeholder: string; tooltip: string; ariaLabel: string }
> = {
  idle: {
    placeholder: 'Ask Zenko anything...',
    tooltip: 'Send prompt',
    ariaLabel: 'Send prompt',
  },
  running_foreground: {
    placeholder: 'Zenko is working — type to queue your next prompt...',
    tooltip: 'Queue follow-up prompt',
    ariaLabel: 'Queue prompt',
  },
  running_orchestrating: {
    placeholder: 'Managing background tasks — type to steer live...',
    tooltip: 'Steer orchestrator live',
    ariaLabel: 'Steer active turn',
  },
}

/**
 * Elevated Zenko Chat Input Box — customizable via Appearance Studio (`ui.components.composer`),
 * strictly inheriting the global theme colors. Uses automatic async dispatch (Queue during
 * focused foreground work, Steer while orchestrating background jobs or subagents).
 */
export const PromptComposer = memo(function PromptComposer({
  value,
  onChange,
  onSend,
  onStop,
  queuedPrompts = [],
  onCancelQueued,
  onPromoteQueued,
  isRunning,
  isOrchestratingBg = false,
  activeProvider,
  activeModel,
  providers,
  configuredProviders,
  models,
  onSelectProviderModel,
  permissionMode,
  onChangePermission,
  streamingEnabled,
  onToggleStreaming,
  activeSessionId,
  totalTokens,
  branchEditBanner,
  onCancelBranchEdit,
  onOpenSettings,
  composerStyle,
  popoverStyle = 'gooey',
  themeMode = 'dark',
  extraTools,
  compactWrapper = false,
}: {
  value: string
  onChange: (v: string) => void
  onSend: () => void
  onStop: () => void
  queuedPrompts?: QueuedPrompt[]
  onCancelQueued?: (qid: string) => void
  onPromoteQueued?: (qid: string) => void
  isRunning: boolean
  isOrchestratingBg?: boolean
  activeProvider: string
  activeModel: string
  providers: ProviderDescriptor[]
  configuredProviders: Record<string, boolean>
  models: ModelInfo[]
  onSelectProviderModel: (providerId: string, modelId: string) => void
  permissionMode: PermissionMode
  onChangePermission: (p: PermissionMode) => void
  streamingEnabled: boolean
  onToggleStreaming?: () => void
  activeSessionId?: string | null
  totalTokens: number
  branchEditBanner: boolean
  onCancelBranchEdit: () => void
  onOpenSettings?: () => void
  composerStyle: ComponentSlots['composer']
  popoverStyle?: 'gooey' | 'crisp'
  themeMode?: 'dark' | 'light'
  extraTools?: ReactNode
  compactWrapper?: boolean
}) {
  const textareaRef = useRef<HTMLTextAreaElement>(null)
  const [composerFocused, setComposerFocused] = useState(false)
  const [modelOpen, setModelOpen] = useState(false)
  const [policyOpen, setPolicyOpen] = useState(false)
  const [contextOpen, setContextOpen] = useState(false)
  const [contextData, setContextData] = useState<ContextBreakdown | null>(null)
  const [loadingContext, setLoadingContext] = useState(false)

  const refreshContextBreakdown = async (sid: string) => {
    setLoadingContext(true)
    try {
      const data = await api<ContextBreakdown>(`/api/sessions/${encodeURIComponent(sid)}/context`)
      setContextData(data)
    } catch {
      setContextData(null)
    } finally {
      setLoadingContext(false)
    }
  }

  useEffect(() => {
    setContextOpen(false)
    setContextData(null)
  }, [activeSessionId])

  useEffect(() => {
    const el = textareaRef.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = `${Math.min(180, Math.max(42, el.scrollHeight))}px`
  }, [value])

  const modelMeta = models.find((m) => m.id === activeModel)
  const contextLimit = modelMeta?.context_length ?? null
  const usageRatio = contextLimit ? Math.min(100, (totalTokens / contextLimit) * 100) : 0
  const meterChrome: ContextMeterChrome =
    composerStyle.context_meter_chrome || 'bordered_filled'
  const meterChromeClass =
    CONTEXT_METER_CHROME_CLASS[meterChrome] || CONTEXT_METER_CHROME_CLASS.bordered_filled

  const shellClass = SHELL_VARIANT_CLASS[composerStyle.shell_variant] || SHELL_VARIANT_CLASS.floating_card
  const pillClass = ACTION_PILL_CLASS[composerStyle.action_pills] || ACTION_PILL_CLASS.soft_tint
  const sendShapeClass = SEND_SHAPE_CLASS[composerStyle.send_button_shape] || SEND_SHAPE_CLASS.rounded_square
  const SendGlyph = SEND_ICON_MAP[composerStyle.send_button_icon] || ArrowUp
  const dispatchState: ComposerDispatchState = !isRunning
    ? 'idle'
    : isOrchestratingBg
      ? 'running_orchestrating'
      : 'running_foreground'
  const modeMeta = COMPOSER_STATE_META[dispatchState]

  const leftToolbarControls = (
    <>
      {composerStyle.show_attach && (
        <ZenkoTooltip label="Insert workspace path" side="top">
          <button
            type="button"
            aria-label="Insert workspace path hint"
            onClick={() => onChange(value ? `${value} @` : 'Read and inspect ')}
            className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-lg text-[var(--muted)] transition-colors hover:bg-[var(--card)] hover:text-[var(--fg)]"
          >
            <ZenkoIcon slot="attach" size={14} />
          </button>
        </ZenkoTooltip>
      )}

      {/* Two-Level Provider -> Live Models Picker (or RareUI Morph Picker) */}
      <ModelPicker
        activeProvider={activeProvider}
        activeModel={activeModel}
        providers={providers}
        configuredProviders={configuredProviders}
        activeModels={models}
        pickerStyle={composerStyle.model_picker_style}
        triggerChrome={composerStyle.model_trigger_chrome}
        pillClass={pillClass}
        open={modelOpen}
        onOpenChange={(next) => {
          setModelOpen(next)
          if (next) setPolicyOpen(false)
        }}
        onSelectProviderModel={onSelectProviderModel}
        onOpenSettings={onOpenSettings}
      />

      {/* Execution Policy Pill powered by BeUI Popover */}
      {composerStyle.show_policy && (
        <ZenkoTooltip label="Execution policy" side="top" disabled={policyOpen}>
          <Popover
            open={policyOpen}
            onOpenChange={(next) => {
              setPolicyOpen(next)
              if (next) {
                setModelOpen(false)
                setContextOpen(false)
              }
            }}
            side="top"
            align="start"
            sideOffset={10}
            panelRadius={14}
            gooStrength={popoverStyle === 'gooey' ? 7 : 0}
          >
            <PopoverTrigger>
              <button
                type="button"
                aria-label="Execution policy"
                className={cn(
                  'inline-flex h-7 cursor-pointer items-center gap-1.5 rounded-full px-2.5 text-[11.5px] font-medium transition-colors',
                  pillClass,
                )}
              >
                <ZenkoIcon slot="zap" size={11.5} className="text-[var(--muted)]" />
                <span className="capitalize">{permissionMode}</span>
              </button>
            </PopoverTrigger>
            <PopoverContent className="w-56 rounded-[14px] border border-[var(--border)] bg-[var(--surface)] p-1.5 shadow-2xl">
              <div className="flex flex-col gap-0.5">
                {POLICY_OPTIONS.map(([id, title, desc]) => (
                  <button
                    key={id}
                    type="button"
                    onClick={() => {
                      onChangePermission(id)
                      setPolicyOpen(false)
                    }}
                    className={cn(
                      'flex w-full cursor-pointer flex-col rounded-lg px-2.5 py-1.5 text-left transition-colors hover:bg-[var(--card)]',
                      permissionMode === id && 'bg-[var(--card)]',
                    )}
                  >
                    <span className="flex items-center justify-between text-[12px] font-medium text-[var(--fg)]">
                      {title}
                      {permissionMode === id && (
                        <span className="h-1.5 w-1.5 rounded-full bg-[var(--accent)]" />
                      )}
                    </span>
                    <span className="text-[10.5px] text-[var(--muted)]">{desc}</span>
                  </button>
                ))}
              </div>
            </PopoverContent>
          </Popover>
        </ZenkoTooltip>
      )}

      {/* Live Interactive Streaming Toggle */}
      {composerStyle.show_streaming_toggle && (
        <ZenkoTooltip label="Toggle live SSE streaming" side="top">
          <button
            type="button"
            onClick={onToggleStreaming}
            className={cn(
              'hidden h-7 cursor-pointer items-center gap-1.5 rounded-full px-2.5 text-[11.5px] font-medium transition-colors sm:inline-flex',
              pillClass,
            )}
          >
            <span className={streamingEnabled ? 'text-[var(--fg)]' : 'text-[var(--muted)]'}>
              {streamingEnabled ? 'Stream ON' : 'Stream OFF'}
            </span>
          </button>
        </ZenkoTooltip>
      )}
    </>
  )

  const smoothBreakdown = contextData
    ? [
        { label: 'Tier 1 (KV frozen)', tokens: contextData.tier1_tokens },
        { label: 'Tier 2 (Memory/Skills)', tokens: contextData.tier2_tokens },
        { label: 'Tool Schemas', tokens: contextData.tool_schema_tokens },
        { label: 'Active History', tokens: contextData.history_tokens },
      ]
    : [
        { label: 'Session Used', tokens: totalTokens },
        {
          label: 'Available Window',
          tokens: Math.max(0, (contextLimit ?? 128000) - totalTokens),
        },
      ]

  const sendActionControl = isRunning ? (
    <div className="flex items-center gap-1.5">
      {value.trim() && (
        <ZenkoTooltip label={modeMeta.tooltip} shortcut="⏎" side="top">
          <motion.button
            type="button"
            whileTap={{ scale: 0.92 }}
            transition={SPRING_PRESS}
            onClick={onSend}
            aria-label={modeMeta.ariaLabel}
            className={cn(
              'z-btn z-btn-primary flex items-center justify-center',
              sendShapeClass,
            )}
          >
            <SendGlyph size={14} strokeWidth={2.2} />
            {composerStyle.send_button_shape === 'pill_label' && (
              <span>{isOrchestratingBg ? 'Steer' : 'Queue'}</span>
            )}
          </motion.button>
        </ZenkoTooltip>
      )}
      <ZenkoTooltip label="Stop generation" side="top">
        <motion.button
          type="button"
          whileTap={{ scale: 0.92 }}
          transition={SPRING_PRESS}
          onClick={onStop}
          aria-label="Stop generation"
          className={cn(
            'z-btn z-btn-danger flex items-center justify-center',
            sendShapeClass,
          )}
        >
          <ZenkoIcon slot="stop" size={13} />
          {composerStyle.send_button_shape === 'pill_label' && <span>Stop</span>}
        </motion.button>
      </ZenkoTooltip>
    </div>
  ) : (
    <ZenkoTooltip label="Send prompt" shortcut="⏎" side="top">
      <motion.button
        type="button"
        whileTap={{ scale: 0.92 }}
        transition={SPRING_PRESS}
        disabled={!value.trim()}
        onClick={onSend}
        aria-label="Send prompt"
        className={cn(
          'z-btn z-btn-primary flex items-center justify-center',
          sendShapeClass,
        )}
      >
        <SendGlyph size={15} strokeWidth={2.2} />
        {composerStyle.send_button_shape === 'pill_label' && <span>Send</span>}
      </motion.button>
    </ZenkoTooltip>
  )

  const kokonutSegments = contextData
    ? [
        { id: 'tier1', label: 'Tier 1 (KV frozen)', tokens: contextData.tier1_tokens },
        { id: 'tier2', label: 'Tier 2 (Memory/Skills)', tokens: contextData.tier2_tokens },
        { id: 'tools', label: 'Tool Schemas', tokens: contextData.tool_schema_tokens },
        { id: 'history', label: 'Active History', tokens: contextData.history_tokens },
      ]
    : [
        { id: 'used', label: 'Session Used', tokens: totalTokens },
      ]

  const contextMeterControl =
    composerStyle.context_meter === 'hidden' ? null : composerStyle.context_meter ===
      'kokonut_meter' ? (
      <div className="inline-flex">
        <KokonutContextMeter
          compact
          chrome={meterChrome}
          segments={kokonutSegments}
          limit={contextLimit ?? 128000}
          model={activeModel}
          onOpenChange={(next) => {
            if (next && activeSessionId) refreshContextBreakdown(activeSessionId)
          }}
        />
      </div>
    ) : (
    <ZenkoTooltip
      label="Inspect live context & KV-cache breakdown"
      side="top"
      disabled={contextOpen || composerStyle.context_meter === 'smoothui_meter'}
    >
      <Popover
        open={contextOpen}
        onOpenChange={(next) => {
          setContextOpen(next)
          if (next) {
            setModelOpen(false)
            setPolicyOpen(false)
            if (activeSessionId) refreshContextBreakdown(activeSessionId)
          }
        }}
        side="top"
        align="end"
        sideOffset={10}
        panelRadius={14}
        gooStrength={popoverStyle === 'gooey' ? 7 : 0}
      >
        <PopoverTrigger>
          {composerStyle.context_meter === 'smoothui_meter' ? (
            <div className="inline-flex">
              <AIContextMeter
                used={totalTokens}
                limit={contextLimit ?? 128000}
                chrome={meterChrome}
                breakdown={smoothBreakdown}
              />
            </div>
          ) : (
            <button
              type="button"
              aria-label="Inspect live context & KV-cache breakdown"
              className={cn(
                'inline-flex cursor-pointer items-center gap-1.5 font-mono text-[11px] text-[var(--muted)] transition-colors hover:text-[var(--fg)]',
                meterChromeClass,
              )}
            >
              {composerStyle.context_meter === 'progress_ring' && contextLimit != null && (
                <ProgressRing value={totalTokens} max={contextLimit} size={13} />
              )}
              {composerStyle.context_meter === 'motion_brew_ring' && (
                <CompactSegmentedRing ratio={usageRatio} />
              )}
              {composerStyle.context_meter === 'linear_bar' && (
                <div className="h-1.5 w-14 overflow-hidden rounded-full bg-[var(--border)]">
                  <div
                    className="h-full rounded-full transition-all"
                    style={{ width: `${usageRatio}%`, backgroundColor: 'var(--accent)' }}
                  />
                </div>
              )}
              <span>
                {formatCompactTokens(totalTokens)}
                {contextLimit != null ? ` / ${formatCompactTokens(contextLimit)}` : ' tok'}
              </span>
            </button>
          )}
        </PopoverTrigger>
        <PopoverContent className="w-80 rounded-[14px] border border-[var(--border)] bg-[var(--surface)] p-3.5 text-[11.5px] shadow-2xl">
          <div className="mb-2.5 flex items-center justify-between">
            <span className="font-medium text-[var(--fg)]">Context Breakdown</span>
            <div className="flex items-center gap-2 text-[var(--muted)]">
              {activeSessionId && (
                <ZenkoTooltip label="Refresh breakdown" side="top">
                  <button
                    type="button"
                    aria-label="Refresh breakdown"
                    onClick={() => refreshContextBreakdown(activeSessionId)}
                    className="cursor-pointer hover:text-[var(--fg)]"
                  >
                    <RefreshCw size={11.5} className={loadingContext ? 'animate-spin' : ''} />
                  </button>
                </ZenkoTooltip>
              )}
              <button
                type="button"
                aria-label="Close context breakdown"
                onClick={() => setContextOpen(false)}
                className="cursor-pointer hover:text-[var(--fg)]"
              >
                <X size={12} />
              </button>
            </div>
          </div>

          {!activeSessionId ? (
            <p className="text-[var(--muted)]">Start a session to inspect live token tiers.</p>
          ) : loadingContext && !contextData ? (
            <p className="font-mono text-[var(--muted)]">Measuring context...</p>
          ) : contextData ? (
            <div className="space-y-2.5">
              <div>
                <div className="mb-1 flex items-center justify-between font-mono text-[11px]">
                  <span className="text-[var(--fg)]">
                    {formatCompactTokens(contextData.total_tokens)} /{' '}
                    {formatCompactTokens(contextData.context_window)}
                  </span>
                  <span className="text-[var(--accent)]">
                    {contextData.utilization_pct.toFixed(1)}%
                  </span>
                </div>
                <div className="h-1.5 w-full overflow-hidden rounded-full bg-[var(--border)]">
                  <div
                    className="h-full rounded-full transition-all"
                    style={{
                      width: `${Math.min(100, contextData.utilization_pct)}%`,
                      backgroundColor: 'var(--accent)',
                    }}
                  />
                </div>
              </div>

              <div className="space-y-1 border-t border-[var(--border)]/60 pt-2 font-mono text-[11px]">
                {[
                  ['Tier 1 (KV frozen)', `${formatCompactTokens(contextData.tier1_tokens)} tok`],
                  ['Tier 2 (Memory/Skills)', `${formatCompactTokens(contextData.tier2_tokens)} tok`],
                  [
                    'Tools',
                    `${formatCompactTokens(contextData.tool_schema_tokens)} tok (${contextData.eager_tools_count}+${contextData.deferred_active_count}/${contextData.deferred_available_count})`,
                  ],
                  ['Active History', `${formatCompactTokens(contextData.history_tokens)} tok`],
                  ['KV Cache Hit', `${contextData.cache_hit_pct.toFixed(1)}%`],
                  [
                    'Microcompacted',
                    `${contextData.microcompacted_tools_count} (${formatCompactTokens(contextData.microcompacted_tokens_saved)} saved)`,
                  ],
                  ['Checkpoints', `${contextData.checkpoints_count}`],
                  ['Spilled Artifacts', `${contextData.spilled_artifacts_count}`],
                ].map(([label, val]) => (
                  <div key={label} className="flex items-center justify-between gap-2">
                    <span className="text-[var(--muted)]">{label}</span>
                    <span className="text-[var(--fg)]">{val}</span>
                  </div>
                ))}
              </div>

              {contextData.repo_rules_loaded.length > 0 && (
                <div className="border-t border-[var(--border)]/60 pt-2 font-mono text-[10.5px] text-[var(--muted)]">
                  Rules: {contextData.repo_rules_loaded.join(', ')}
                </div>
              )}
            </div>
          ) : (
            <p className="text-[var(--muted)]">Context details unavailable.</p>
          )}
        </PopoverContent>
      </Popover>
    </ZenkoTooltip>
  )

  return (
    <div className={compactWrapper ? 'w-full' : 'px-4 pb-5 pt-2 sm:px-8'}>
      <div className={compactWrapper ? 'relative w-full' : 'relative mx-auto w-full max-w-[780px]'}>
        {branchEditBanner && (
          <div
            className="mb-2 flex items-center justify-between rounded-xl border px-3 py-1.5 text-[12px]"
            style={{
              borderColor: 'color-mix(in srgb, var(--accent) 42%, var(--border))',
              backgroundColor: 'color-mix(in srgb, var(--accent) 10%, var(--card))',
            }}
          >
            <span>Editing & branching from earlier message...</span>
            <button
              type="button"
              onClick={onCancelBranchEdit}
              className="cursor-pointer text-[var(--muted)] hover:text-[var(--fg)]"
            >
              <X size={13} />
            </button>
          </div>
        )}

        {queuedPrompts.length > 0 && (
          <div className="mb-2 flex flex-wrap items-center gap-1.5">
            {queuedPrompts.map((qp, idx) => (
              <div
                key={qp.id}
                className="inline-flex max-w-full items-center gap-2 rounded-full border border-[var(--border)] bg-[var(--surface)] px-2.5 py-1 text-[11px] text-[var(--fg)]"
              >
                <span className="font-mono text-[var(--muted)]">#{idx + 1} queued</span>
                <span className="max-w-[240px] truncate text-[var(--fg)]/88" title={qp.text}>
                  {qp.text}
                </span>
                {isRunning && onPromoteQueued && (
                  <ZenkoTooltip label="Inject into active turn right now" side="top">
                    <button
                      type="button"
                      onClick={() => onPromoteQueued(qp.id)}
                      className="cursor-pointer font-mono text-[10px] text-[var(--accent)] hover:underline"
                    >
                      Steer now
                    </button>
                  </ZenkoTooltip>
                )}
                {onCancelQueued && (
                  <ZenkoTooltip label="Cancel queued follow-up" side="top">
                    <button
                      type="button"
                      onClick={() => onCancelQueued(qp.id)}
                      className="cursor-pointer text-[var(--muted)] hover:text-[var(--destructive)]"
                    >
                      <X size={11.5} />
                    </button>
                  </ZenkoTooltip>
                )}
              </div>
            ))}
          </div>
        )}

        <ComposerBeam
          animation={composerStyle.border_animation}
          trigger={composerStyle.animation_trigger}
          colorVariant={composerStyle.beam_color_variant}
          strength={composerStyle.beam_strength}
          shellVariant={composerStyle.shell_variant}
          isWorking={isRunning || isOrchestratingBg}
          isFocused={composerFocused}
          themeMode={themeMode}
        >
        {composerStyle.shell_variant === 'motion_brew_prompt' ? (
          <div
            onFocusCapture={() => setComposerFocused(true)}
            onBlurCapture={() => setComposerFocused(false)}
            className="w-full"
          >
          <MotionBrewPromptInput
            value={value}
            onValueChange={onChange}
            isWorking={isRunning}
            onStop={onStop}
            onSubmit={() => onSend()}
            placeholders={[
              modeMeta.placeholder,
              'Inspect workspace architecture & check crate tests...',
              'Search memory snapshots and active playbooks...',
            ]}
            leftToolbar={leftToolbarControls}
            rightToolbar={
              <>
                {extraTools}
                {contextMeterControl}
              </>
            }
            sendButton={sendActionControl}
            className="w-full"
          />
          </div>
        ) : (
        <div
          onFocusCapture={() => setComposerFocused(true)}
          onBlurCapture={() => setComposerFocused(false)}
          className={cn('transition-all duration-150', shellClass)}
        >
          <SmoothCaretTextarea
            ref={textareaRef}
            rows={1}
            value={value}
            onChange={(e) => onChange(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
                e.preventDefault()
                if (!value.trim()) return
                onSend()
              }
            }}
            placeholder={modeMeta.placeholder}
            className="w-full resize-none bg-transparent text-[14.5px] leading-relaxed text-[var(--fg)] placeholder:text-[var(--muted)]/60 focus:outline-none"
          />

          <div className="mt-2.5 flex flex-wrap items-center justify-between gap-2 border-t border-[var(--border)]/50 pt-2.5">
            <div className="flex flex-wrap items-center gap-1.5">
              {leftToolbarControls}
            </div>

            {/* Right: Workspace Tools (Tasks / Checkpoints) + Context Meter + Send / Stop */}
            <div className="relative flex items-center gap-1.5">
              {extraTools}
              {contextMeterControl}
              {sendActionControl}
            </div>
          </div>
        </div>
        )}
        </ComposerBeam>
      </div>
    </div>
  )
})
