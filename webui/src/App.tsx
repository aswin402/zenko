import { Suspense, lazy, useCallback, useEffect, useRef, useState } from 'react'
import { AnimatePresence, motion } from 'motion/react'
import { Search, X } from 'lucide-react'
import { api, applyUiCustomization, normalizeUiCustomization } from '@/lib/api'
import { SPRING_PANEL } from '@/lib/ease'
import { appendBoundedTerminal } from '@/lib/terminal'
import type {
  ActiveView,
  ApprovalRecord,
  BackgroundJob,
  BootstrapResponse,
  ClarificationRecord,
  DaemonTelemetry,
  HubCategoryMeta,
  LogCategoryMeta,
  LogEntry,
  LspServerDescriptor,
  LspServerRuntimeStatus,
  McpServerStatus,
  ModelInfo,
  OrchestratorStatus,
  PermissionMode,
  ProviderDescriptor,
  QueuedPrompt,
  RewindMode,
  RewindReport,
  Session,
  SessionEvent,
  SessionTask,
  StudioCategory,
  SubagentRecord,
  SubagentRoleDescriptor,
  TaskKindMeta,
  ThemePresetMeta,
  UiCustomization,
  UiProfile,
  WebSearchProviderDescriptor,
  WorkspaceCheckpoint,
} from '@/lib/types'
import { cn } from '@/lib/utils'
import { FloatingDock } from '@/components/beui/FloatingDock'
import { ZenkoIconProvider } from '@/components/beui/ZenkoIcon'
import {
  Alert as EaseAlert,
  AlertDescription as EaseAlertDescription,
} from '@/components/easeui/alert'
import {
  CommandEmpty as EaseCommandEmpty,
  CommandGroup as EaseCommandGroup,
  CommandInput as EaseCommandInput,
  CommandItem as EaseCommandItem,
  CommandList as EaseCommandList,
  CommandPalette as EaseCommandPalette,
  CommandSeparator as EaseCommandSeparator,
} from '@/components/easeui/command-palette'
import { Toaster as EaseToaster, toast as easeToast } from '@/components/easeui/toast'
import {
  Alert,
  AlertContent,
  AlertDescription,
  AlertIcon,
} from '@/components/motion/alert'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
} from '@/components/motion/center-morph-modal'
import { ThemeToggle } from '@/components/motion/theme-toggle'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { FloatingActivityMonitor } from '@/components/ui/FloatingActivityMonitor'
import { ThemeSelector } from '@/components/ui/ThemeSelector'
import { ZenkoTooltipProvider } from '@/components/ui/Tooltip'
import { Sidebar } from '@/components/chat/Sidebar'
import { ChatStream, type LiveToolItem } from '@/components/chat/ChatStream'
import { Phase6Bar, WorkspaceToolsPopovers } from '@/components/chat/Phase6Bar'
import { PromptComposer } from '@/components/chat/PromptComposer'
import { LoginGate } from '@/components/views/LoginGate'

const AppearanceStudio = lazy(() =>
  import('@/components/views/AppearanceStudio').then((m) => ({ default: m.AppearanceStudio })),
)
const DashboardView = lazy(() =>
  import('@/components/views/DashboardView').then((m) => ({ default: m.DashboardView })),
)
const SetupView = lazy(() =>
  import('@/components/views/SetupView').then((m) => ({ default: m.SetupView })),
)
const GatewaysView = lazy(() =>
  import('@/components/views/GatewaysView').then((m) => ({ default: m.GatewaysView })),
)
const MemoryView = lazy(() =>
  import('@/components/views/MemoryView').then((m) => ({ default: m.MemoryView })),
)
const AutomationView = lazy(() =>
  import('@/components/views/AutomationView').then((m) => ({ default: m.AutomationView })),
)

interface ToastItem {
  id: number
  message: string
  kind: 'info' | 'success' | 'error'
}

const TOAST_POSITION_STACK_CLASS: Record<
  NonNullable<UiCustomization['surface']['toast_position']>,
  string
> = {
  top_center: 'top-4 left-1/2 -translate-x-1/2 items-center',
  top_right: 'top-4 right-5 items-end',
  top_left: 'top-4 left-5 items-start',
  bottom_center: 'bottom-5 left-1/2 -translate-x-1/2 items-center',
  bottom_right: 'bottom-5 right-5 items-end',
  bottom_left: 'bottom-5 left-5 items-start',
}

const COMMAND_PALETTE_VIEWS: ReadonlyArray<[ActiveView, string]> = [
  ['chat', 'Chat Workbench'],
  ['gateways', 'Telegram & Gateways'],
  ['memory', 'Memory & Prompt Snapshots'],
  ['automation', 'Cron, Heartbeats & Policy'],
  ['telemetry', 'Dashboard'],
  ['appearance', 'Customization'],
  ['setup', 'Settings & Providers Setup'],
]

export default function App() {
  const [authorized, setAuthorized] = useState(true)
  const [loaded, setLoaded] = useState(false)
  const [activeView, setActiveView] = useState<ActiveView>('chat')
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [dockCollapsed, setDockCollapsed] = useState(false)
  const [monitorOpen, setMonitorOpen] = useState(false)
  const [terminalBuffers, setTerminalBuffers] = useState<Record<string, string>>({})
  const [quickCustomizerOpen, setQuickCustomizerOpen] = useState(false)
  const [cmdKOpen, setCmdKOpen] = useState(false)
  const [clearConfirmOpen, setClearConfirmOpen] = useState(false)
  const [cmdQuery, setCmdQuery] = useState('')
  const [studioJumpTarget, setStudioJumpTarget] = useState<{
    categoryId: string
    subId: string
  } | null>(null)

  const [config, setConfig] = useState<any>(null)
  const [telemetry, setTelemetry] = useState<DaemonTelemetry | undefined>()
  const [sessions, setSessions] = useState<Session[]>([])
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null)
  const [events, setEvents] = useState<SessionEvent[]>([])
  const [siblings, setSiblings] = useState<Record<string, string[]>>({})
  const [pendingApprovals, setPendingApprovals] = useState<ApprovalRecord[]>([])
  const [pendingClarifications, setPendingClarifications] = useState<ClarificationRecord[]>([])
  const [liveOrchestrator, setLiveOrchestrator] = useState<OrchestratorStatus | null>(null)
  const [models, setModels] = useState<ModelInfo[]>([])
  const [logs, setLogs] = useState<LogEntry[]>([])
  const [jobs, setJobs] = useState<BackgroundJob[]>([])
  const [subagents, setSubagents] = useState<SubagentRecord[]>([])
  const [subagentRoles, setSubagentRoles] = useState<SubagentRoleDescriptor[]>([])
  const [queuedPrompts, setQueuedPrompts] = useState<QueuedPrompt[]>([])
  const [sessionTasks, setSessionTasks] = useState<SessionTask[]>([])
  const [sessionCheckpoints, setSessionCheckpoints] = useState<WorkspaceCheckpoint[]>([])
  const [logCategories, setLogCategories] = useState<LogCategoryMeta[]>([])
  const [logCounts, setLogCounts] = useState<Record<string, number>>({})
  const [logsDir, setLogsDir] = useState<string>('')
  const [taskKinds, setTaskKinds] = useState<TaskKindMeta[]>([])
  const [hubDir, setHubDir] = useState<string>('')
  const [hubCategories, setHubCategories] = useState<HubCategoryMeta[]>([])

  const [ui, setUi] = useState<UiCustomization | null>(null)
  const [presets, setPresets] = useState<ThemePresetMeta[]>([])
  const [studioSchema, setStudioSchema] = useState<StudioCategory[]>([])
  const [providers, setProviders] = useState<ProviderDescriptor[]>([])
  const [configuredProviders, setConfiguredProviders] = useState<Record<string, boolean>>({})
  const [hasProviderKey, setHasProviderKey] = useState(false)
  const [hasTelegram, setHasTelegram] = useState(false)
  const [hasBraveSearchKey, setHasBraveSearchKey] = useState(false)
  const [hasTavilySearchKey, setHasTavilySearchKey] = useState(false)
  const [webSearchProviders, setWebSearchProviders] = useState<WebSearchProviderDescriptor[]>([])
  const [mcpServers, setMcpServers] = useState<McpServerStatus[]>([])
  const [lspRegistry, setLspRegistry] = useState<LspServerDescriptor[]>([])
  const [lspActive, setLspActive] = useState<LspServerRuntimeStatus[]>([])

  // Live streaming state
  const [composerText, setComposerText] = useState('')
  const [draftPolicy, setDraftPolicy] = useState<PermissionMode | null>(null)
  const [branchParent, setBranchParent] = useState<string | null | undefined>(undefined)
  const [liveThinking, setLiveThinking] = useState('')
  const [liveText, setLiveText] = useState('')
  const [liveTools, setLiveTools] = useState<LiveToolItem[]>([])
  const [isRunning, setIsRunning] = useState(false)
  const [turnStartedAt, setTurnStartedAt] = useState<number | null>(null)
  const [liveTurnError, setLiveTurnError] = useState<{
    title: string
    detail: string
    retrying: boolean
  } | null>(null)
  const [stoppedRun, setStoppedRun] = useState<{
    words: string[]
    reason: string
  } | null>(null)
  const [toasts, setToasts] = useState<ToastItem[]>([])

  const activeSidRef = useRef<string | null>(null)
  activeSidRef.current = activeSessionId
  const uiRef = useRef<UiCustomization | null>(null)
  uiRef.current = ui
  const loadSessionSeqRef = useRef<number>(0)
  const loadSessionAbortRef = useRef<AbortController | null>(null)
  const lastPromptRef = useRef<string>('')
  const liveTextRef = useRef<string>('')
  liveTextRef.current = liveText
  const saveUiTimer = useRef<number | undefined>(undefined)

  useEffect(
    () => () => {
      window.clearTimeout(saveUiTimer.current)
    },
    [],
  )

  const toast = useCallback((message: string, kind: 'info' | 'success' | 'error' = 'info') => {
    if (uiRef.current?.surface.toast_style === 'easeui_toast') {
      if (kind === 'success') easeToast.success(message)
      else if (kind === 'error') easeToast.error(message)
      else easeToast(message)
      return
    }
    const id = Date.now() + Math.random()
    setToasts((prev) => [...prev.slice(-3), { id, message, kind }])
    setTimeout(() => setToasts((prev) => prev.filter((t) => t.id !== id)), 3400)
  }, [])

  const loadSession = useCallback(async (sid: string, opts?: { preserveLive?: boolean }) => {
    const seq = ++loadSessionSeqRef.current
    loadSessionAbortRef.current?.abort()
    const controller = new AbortController()
    loadSessionAbortRef.current = controller
    try {
      const data = await api<{
        session: Session
        events: SessionEvent[]
        siblings: Record<string, string[]>
        pending_approvals: ApprovalRecord[]
        pending_clarifications?: ClarificationRecord[]
        queued_prompts?: QueuedPrompt[]
        tasks?: SessionTask[]
        checkpoints?: WorkspaceCheckpoint[]
      }>(`/api/sessions/${encodeURIComponent(sid)}`, { signal: controller.signal })
      if (seq !== loadSessionSeqRef.current || activeSidRef.current !== sid) {
        return
      }
      const running =
        data.session.status === 'running' || data.session.status === 'waiting_approval'
      setEvents(data.events)
      setSiblings(data.siblings)
      setPendingApprovals(data.pending_approvals)
      setPendingClarifications(data.pending_clarifications || [])
      setQueuedPrompts(data.queued_prompts || [])
      setSessionTasks(data.tasks || [])
      setSessionCheckpoints(data.checkpoints || [])
      setIsRunning(running)
      if (!running) {
        setTurnStartedAt(null)
        setLiveOrchestrator(null)
      }
      if (!opts?.preserveLive || !running) {
        setLiveThinking('')
        setLiveText('')
        setLiveTools([])
      }
      setSessions((prev) =>
        prev.some((s) => s.id === sid)
          ? prev.map((s) => (s.id === sid ? data.session : s))
          : [...prev, data.session],
      )
    } catch (err: any) {
      if (err?.name === 'AbortError') return
      /* session might have been deleted */
    }
  }, [])

  const bootstrap = useCallback(async () => {
    try {
      const data = await api<BootstrapResponse>('/api/bootstrap')
      if (data.active_profile?.config) {
        const normalized = normalizeUiCustomization(data.active_profile.config)
        setUi(normalized)
        applyUiCustomization(normalized)
      }
      setPresets(data.presets || [])
      setStudioSchema(data.studio_schema || [])
      if (!data.authorized) {
        setAuthorized(false)
        setLoaded(true)
        return
      }
      setAuthorized(true)
      setConfig(data.config)
      setTelemetry(data.telemetry)
      setSessions(data.sessions || [])
      setProviders(data.providers || [])
      setConfiguredProviders(data.configured_providers || {})
      setPendingApprovals(data.pending_approvals || [])
      setPendingClarifications(data.pending_clarifications || [])
      setHasProviderKey(Boolean(data.has_provider_key ?? data.has_openrouter_key))
      setHasTelegram(Boolean(data.has_telegram_token))
      setHasBraveSearchKey(Boolean(data.has_brave_search_key))
      setHasTavilySearchKey(Boolean(data.has_tavily_search_key))
      setWebSearchProviders(data.web_search_providers || [])
      setMcpServers(data.mcp_servers || [])
      setLspRegistry(data.lsp_registry || [])
      setLspActive(data.lsp_active || [])
      setLogCategories(data.log_categories || [])
      setLogCounts(data.log_counts || {})
      setLogsDir(data.logs_dir || '')
      setTaskKinds(data.task_kinds || [])
      setHubDir(data.hub_dir || '')
      setHubCategories(data.hub_categories || [])
      setJobs(data.jobs || [])
      setSubagents(data.subagents || [])
      setSubagentRoles(data.subagent_roles || [])

      // Open WebUI cleanly on a fresh New Session canvas unless a session is already active
      const existingSid = activeSidRef.current
      if (existingSid && data.sessions?.some((s) => s.id === existingSid)) {
        setActiveSessionId(existingSid)
        await loadSession(existingSid)
      } else {
        setActiveSessionId(null)
        setEvents([])
        setSessionTasks([])
        setSessionCheckpoints([])
        setQueuedPrompts([])
        setIsRunning(false)
      }
      api<ModelInfo[]>('/api/models').then(setModels).catch(() => {})
      api<LogEntry[]>('/api/logs').then(setLogs).catch(() => {})
      setLoaded(true)
    } catch (e: any) {
      toast(e.message, 'error')
      setLoaded(true)
    }
  }, [loadSession, toast])

  useEffect(() => {
    bootstrap()
  }, [bootstrap])

  // Keyboard shortcuts: Cmd/Ctrl+K (Command Palette) & Cmd/Ctrl+. (Quick Customizer)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'k') {
        e.preventDefault()
        setCmdKOpen((o) => !o)
      } else if ((e.metaKey || e.ctrlKey) && e.key === '.') {
        e.preventDefault()
        setQuickCustomizerOpen((o) => !o)
      } else if (e.key === 'Escape') {
        setCmdKOpen(false)
        setQuickCustomizerOpen(false)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  // Live Daemon SSE Subscription with rAF-coalesced streaming buffers (max 1 React commit/frame)
  useEffect(() => {
    if (!authorized || !loaded) return
    const es = new EventSource('/api/events')

    let pendingThinking = ''
    let pendingText = ''
    let pendingJobChunks: Record<string, { chunk: string; bytes?: number }> = {}
    let pendingLogs: LogEntry[] = []
    let flushRafId: number | null = null

    const flushStreamBuffers = () => {
      flushRafId = null
      if (pendingThinking) {
        const delta = pendingThinking
        pendingThinking = ''
        setLiveThinking((t) => t + delta)
      }
      if (pendingText) {
        const delta = pendingText
        pendingText = ''
        setLiveText((t) => t + delta)
      }
      const jobKeys = Object.keys(pendingJobChunks)
      if (jobKeys.length > 0) {
        const batch = pendingJobChunks
        pendingJobChunks = {}
        setTerminalBuffers((prev) => {
          const next = { ...prev }
          for (const jid of jobKeys) {
            next[jid] = appendBoundedTerminal(next[jid] || '', batch[jid].chunk)
          }
          return next
        })
        const bytesUpdates = jobKeys.filter((jid) => typeof batch[jid].bytes === 'number')
        if (bytesUpdates.length > 0) {
          setJobs((prev) =>
            prev.map((j) =>
              batch[j.id] && typeof batch[j.id].bytes === 'number'
                ? { ...j, bytes_written: batch[j.id].bytes! }
                : j,
            ),
          )
        }
      }
      if (pendingLogs.length > 0) {
        const batch = pendingLogs
        pendingLogs = []
        setLogs((prev) => [...prev.slice(-(240 - Math.min(batch.length, 240))), ...batch])
        setLogCounts((prev) => {
          const next = { ...prev }
          for (const entry of batch) {
            const cat = entry.category || 'system'
            next[cat] = (next[cat] || 0) + 1
          }
          return next
        })
      }
    }

    const scheduleFlush = () => {
      if (flushRafId === null) {
        flushRafId = window.requestAnimationFrame(flushStreamBuffers)
      }
    }

    const discardLiveTextBuffers = () => {
      pendingThinking = ''
      pendingText = ''
    }

    const handlers: Record<string, (ev: any, sid: string | null) => void> = {
      session_updated: (ev, sid) => {
        setSessions((prev) =>
          prev.map((s) =>
            s.id === ev.session_id
              ? { ...s, status: ev.status, title: ev.title ?? s.title }
              : s,
          ),
        )
        if (sid && ev.session_id === sid) {
          const running = ev.status === 'running' || ev.status === 'waiting_approval'
          setIsRunning(running)
          if (running) {
            setTurnStartedAt((prev) => prev ?? Date.now())
          } else {
            discardLiveTextBuffers()
            setTurnStartedAt(null)
            loadSession(sid)
          }
        }
      },
      user_message_accepted: (ev, sid) => {
        if (!sid || ev.session_id !== sid) return
        setTurnStartedAt((prev) => prev ?? Date.now())
        loadSession(sid, { preserveLive: true })
      },
      thinking_delta: (ev, sid) => {
        if (sid && ev.session_id === sid && typeof ev.text === 'string') {
          pendingThinking += ev.text
          scheduleFlush()
        }
      },
      text_delta: (ev, sid) => {
        if (sid && ev.session_id === sid && typeof ev.text === 'string') {
          pendingText += ev.text
          scheduleFlush()
        }
      },
      tool_call_started: (ev, sid) => {
        if (!sid || ev.session_id !== sid) return
        flushStreamBuffers()
        setLiveTools((prev) => [
          ...prev,
          {
            call_id: ev.call_id,
            tool_name: ev.tool_name,
            args_preview: ev.args_preview,
            status: 'running',
          },
        ])
      },
      tool_call_finished: (ev, sid) => {
        if (!sid || ev.session_id !== sid) return
        setLiveTools((prev) =>
          prev.map((t) =>
            t.call_id === ev.call_id
              ? {
                  ...t,
                  status: ev.is_error ? 'error' : 'ok',
                  duration_ms: ev.duration_ms,
                  diff: ev.diff,
                  output_preview: ev.output_preview,
                }
              : t,
          ),
        )
      },
      tool_approval_requested: (ev, sid) => {
        toast(`Approval needed: ${ev.approval.tool_name}`, 'info')
        if (sid && ev.approval.session_id === sid) {
          setPendingApprovals((prev) =>
            prev.some((a) => a.id === ev.approval.id) ? prev : [...prev, ev.approval],
          )
        }
      },
      tool_approval_resolved: (ev) => {
        setPendingApprovals((prev) => prev.filter((a) => a.id !== ev.approval_id))
      },
      turn_completed: (ev, sid) => {
        if (!sid || ev.session_id !== sid) return
        discardLiveTextBuffers()
        setLiveTurnError(null)
        loadSession(sid)
        api<DaemonTelemetry>('/api/telemetry').then(setTelemetry).catch(() => {})
      },
      turn_error: (ev, sid) => {
        if (!sid || ev.session_id !== sid) return
        flushStreamBuffers()
        setIsRunning(false)
        setLiveTurnError({
          title: 'Agent turn interrupted',
          detail: String(ev.message || 'An unexpected error occurred during generation.'),
          retrying: false,
        })
        toast(ev.message, 'error')
      },
      gateway_paired: (ev) => {
        toast(`Telegram paired: @${ev.peer.username || ev.peer.peer_id}`, 'success')
      },
      job_updated: (ev) => {
        if (!ev.job) return
        const updated: BackgroundJob = ev.job
        setJobs((prev) => {
          const exists = prev.some((j) => j.id === updated.id)
          return exists
            ? prev.map((j) => (j.id === updated.id ? updated : j))
            : [updated, ...prev].slice(0, 40)
        })
      },
      job_output_delta: (ev) => {
        if (!ev.job_id || typeof ev.chunk !== 'string') return
        const cur = pendingJobChunks[ev.job_id]
        pendingJobChunks[ev.job_id] = {
          chunk: cur ? cur.chunk + ev.chunk : ev.chunk,
          bytes: typeof ev.bytes_written === 'number' ? ev.bytes_written : cur?.bytes,
        }
        scheduleFlush()
      },
      steering_accepted: (ev) => {
        toast(`Steered active turn: ${ev.preview}`, 'info')
      },
      queue_updated: (ev, sid) => {
        if (!sid || ev.session_id !== sid) return
        api<QueuedPrompt[]>(`/api/sessions/${encodeURIComponent(sid)}/queue`)
          .then(setQueuedPrompts)
          .catch(() => {})
      },
      orchestrator_updated: (ev, sid) => {
        if (!sid || ev.session_id !== sid) return
        setLiveOrchestrator({
          tier: ev.tier || 'standard',
          stage: ev.stage || 'triaged',
          detail: ev.detail || '',
          model: ev.model || null,
        })
      },
      brainstorm_recorded: (ev, sid) => {
        if (!sid || ev.session_id !== sid) return
        loadSession(sid)
      },
      plan_updated: (ev, sid) => {
        if (!sid || ev.session_id !== sid) return
        loadSession(sid)
      },
      clarification_requested: (ev, sid) => {
        if (!ev.clarification) return
        toast(`Decision needed: ${ev.clarification.question}`, 'info')
        if (sid && ev.clarification.session_id === sid) {
          setPendingClarifications((prev) =>
            prev.some((c) => c.id === ev.clarification.id) ? prev : [...prev, ev.clarification],
          )
        }
      },
      clarification_resolved: (ev) => {
        setPendingClarifications((prev) => prev.filter((c) => c.id !== ev.clarification_id))
      },
      subagent_spawned: (ev) => {
        if (!ev.agent) return
        const rec: SubagentRecord = ev.agent
        setSubagents((prev) => [rec, ...prev.filter((a) => a.id !== rec.id)].slice(0, 40))
      },
      subagent_completed: (ev) => {
        if (!ev.agent) return
        const rec: SubagentRecord = ev.agent
        setSubagents((prev) => {
          const exists = prev.some((a) => a.id === rec.id)
          return exists
            ? prev.map((a) => (a.id === rec.id ? rec : a))
            : [rec, ...prev].slice(0, 40)
        })
      },
      session_tasks_updated: (ev, sid) => {
        if (!sid || ev.session_id !== sid || !Array.isArray(ev.tasks)) return
        setSessionTasks(ev.tasks)
      },
      checkpoint_created: (ev, sid) => {
        if (!sid || !ev.checkpoint || ev.checkpoint.session_id !== sid) return
        const cp: WorkspaceCheckpoint = ev.checkpoint
        setSessionCheckpoints((prev) => [cp, ...prev.filter((c) => c.id !== cp.id)].slice(0, 30))
      },
      session_rewound: (ev, sid) => {
        if (!sid || !ev.report || ev.report.session_id !== sid) return
        loadSession(sid)
      },
      plan_approved: (ev, sid) => {
        setSessions((prev) =>
          prev.map((s) =>
            s.id === ev.session_id ? { ...s, permission_mode: ev.new_mode } : s,
          ),
        )
        if (sid && ev.session_id === sid) {
          toast(`Plan approved (${ev.new_mode})`, 'success')
        }
      },
      mcp_servers_updated: (ev) => {
        if (Array.isArray(ev.servers)) {
          setMcpServers(ev.servers)
        }
      },
      log: (ev) => {
        pendingLogs.push(ev)
        if (pendingLogs.length > 120) pendingLogs = pendingLogs.slice(-120)
        scheduleFlush()
      },
    }

    let openedOnce = false
    es.onopen = () => {
      if (openedOnce) {
        api<Session[]>('/api/sessions').then(setSessions).catch(() => {})
        api<BackgroundJob[]>('/api/jobs').then(setJobs).catch(() => {})
        api<DaemonTelemetry>('/api/telemetry').then(setTelemetry).catch(() => {})
        if (activeSidRef.current) {
          loadSession(activeSidRef.current)
        }
      }
      openedOnce = true
    }

    es.onmessage = (msg) => {
      try {
        const ev = JSON.parse(msg.data)
        handlers[ev.event]?.(ev, activeSidRef.current)
      } catch {
        /* ignore malformed SSE frame */
      }
    }
    return () => {
      if (flushRafId !== null) window.cancelAnimationFrame(flushRafId)
      es.close()
    }
  }, [authorized, loaded, loadSession, toast])

  // Live UI Customization patcher (pure state update + synchronous DOM apply + debounced SQLite persist)
  const patchUi = useCallback((mutator: (draft: UiCustomization) => void) => {
    const prev = uiRef.current
    if (!prev) return
    const next: UiCustomization = structuredClone(prev)
    mutator(next)
    uiRef.current = next
    setUi(next)
    applyUiCustomization(next)
    window.clearTimeout(saveUiTimer.current)
    saveUiTimer.current = window.setTimeout(() => {
      api('/api/ui/active', { method: 'PUT', body: JSON.stringify(next) }).catch(() => {})
    }, 220)
  }, [])

  const selectPreset = useCallback(
    async (presetId: string, mode?: 'dark' | 'light') => {
      // Optimistically apply CSS tokens synchronously so BeUI ThemeToggle's
      // View Transitions API circle-blur animation snapshots old vs new DOM cleanly!
      const family = presets.find((p) => p.id === presetId)
      const prev = uiRef.current
      if (family && prev) {
        const targetMode: 'dark' | 'light' = mode ?? (prev.mode === 'light' ? 'light' : 'dark')
        const sw = targetMode === 'light' ? family.light : family.dark
        const isLight = targetMode === 'light'
        const optimistic: UiCustomization = {
          ...prev,
          mode: targetMode,
          preset_id: family.id,
          colors: {
            background: sw.bg,
            surface: sw.surface,
            card: sw.card,
            border: sw.border,
            foreground: sw.fg,
            muted: sw.muted,
            primary: sw.primary,
            primary_foreground: sw.primary_fg,
            accent: sw.primary,
            destructive: isLight ? '#dc2626' : '#ef4444',
            success: isLight ? '#16a34a' : '#22c55e',
            warning: isLight ? '#d97706' : '#f59e0b',
          },
        }
        uiRef.current = optimistic
        setUi(optimistic)
        applyUiCustomization(optimistic)
      }

      try {
        const qs = mode ? `?mode=${encodeURIComponent(mode)}` : ''
        const res = await api<UiProfile>(
          `/api/ui/preset/${encodeURIComponent(presetId)}${qs}`,
          { method: 'POST' },
        )
        if (res.config) {
          const normalized = normalizeUiCustomization(res.config)
          uiRef.current = normalized
          setUi(normalized)
          applyUiCustomization(normalized)
          const label =
            presets.find((p) => p.id === normalized.preset_id)?.label ||
            normalized.preset_id.split('_').join(' ')
          toast(`Theme: ${label} (${normalized.mode})`, 'success')
        }
      } catch (e: any) {
        toast(e.message, 'error')
      }
    },
    [presets, toast],
  )

  const activeSession = sessions.find((s) => s.id === activeSessionId) || null
  const activeProvider = config?.provider?.kind || providers[0]?.id || 'openrouter'
  const activeModel =
    activeSession?.model_override || config?.provider?.lead_model || models[0]?.id || ''
  const activePolicy: PermissionMode =
    activeSession?.permission_mode || draftPolicy || config?.policy?.mode || 'autonomous'
  const streamingEnabled = config?.provider?.enable_streaming ?? true

  const sendMessage = async (overrideText?: string, overrideParentId?: string | null) => {
    const text = (overrideText ?? composerText).trim()
    if (!text) return
    lastPromptRef.current = text
    setLiveTurnError(null)
    setStoppedRun(null)
    const wasRunning = isRunning
    const restoreComposer = overrideText === undefined ? composerText : undefined
    let sid = activeSessionId
    try {
      if (!sid) {
        const created = await api<Session>('/api/sessions', {
          method: 'POST',
          body: JSON.stringify({
            title: 'New Session',
            ...(draftPolicy ? { permission_mode: draftPolicy } : {}),
            ...(activeModel ? { model_override: activeModel } : {}),
          }),
        })
        setDraftPolicy(null)
        setSessions((prev) => [created, ...prev])
        // Update ref synchronously BEFORE POST /messages so early SSE deltas are never dropped!
        activeSidRef.current = created.id
        setActiveSessionId(created.id)
        sid = created.id
      }
      if (!wasRunning) {
        setIsRunning(true)
        setTurnStartedAt(Date.now())
        setLiveThinking('')
        setLiveText('')
        setLiveTools([])
        setLiveOrchestrator(null)
      }
      const payload: any = {
        text,
        mode: 'auto',
        ...(activeModel ? { model: activeModel } : {}),
      }
      const effectiveParent = overrideParentId !== undefined ? overrideParentId : branchParent
      if (effectiveParent !== undefined) {
        payload.parent_event_id = effectiveParent
        if (overrideParentId === undefined) setBranchParent(undefined)
      }
      const res = await api<{
        steered?: boolean
        queued?: boolean
        queued_count?: number
        dispatch_mode?: string
      }>(`/api/sessions/${encodeURIComponent(sid)}/messages`, {
        method: 'POST',
        body: JSON.stringify(payload),
      })
      if (overrideText === undefined) {
        setComposerText('')
      }
      if (res.steered) {
        toast('Steered active orchestrator live', 'info')
      } else if (res.queued) {
        toast(`Queued follow-up (#${res.queued_count ?? 1})`, 'info')
      }
    } catch (e: any) {
      if (!wasRunning) {
        setIsRunning(false)
        setTurnStartedAt(null)
      }
      if (restoreComposer !== undefined) {
        setComposerText((cur) => (cur === '' ? restoreComposer : cur))
      }
      toast(e?.message || 'Failed to send message', 'error')
    }
  }

  const runningJobsCount = jobs.filter((j) => j.status === 'running').length
  const runningSubagentsCount = subagents.filter((a) => a.status === 'running').length
  const activeBgCount = runningJobsCount + runningSubagentsCount
  const isOrchestratingBg = Boolean(
    activeSessionId &&
      (jobs.some((j) => j.status === 'running' && j.session_id === activeSessionId) ||
        subagents.some((a) => a.status === 'running' && a.parent_session_id === activeSessionId) ||
        liveTools.some(
          (t) =>
            t.status === 'running' &&
            (t.tool_name.startsWith('job_') ||
              t.tool_name.startsWith('agent_') ||
              t.tool_name === 'spawn_agent' ||
              t.tool_name === 'task_manage'),
        )),
  )

  if (!loaded || !ui) {
    return <div className="flex h-full items-center justify-center bg-[var(--bg)] text-sm text-[var(--muted)]">Loading Zenko...</div>
  }

  if (!authorized) {
    return <LoginGate onLoggedIn={bootstrap} />
  }

  return (
    <ZenkoIconProvider value={ui.icons}>
    <ZenkoTooltipProvider surface={ui.surface}>
    <div className="zenko-canvas flex h-full w-full overflow-hidden bg-[var(--bg)] text-[var(--fg)]">
      {/* Left Minimal Session Sidebar — hides automatically in full-page Component Studio */}
      {activeView !== 'appearance' && (
        <Sidebar
          open={sidebarOpen}
          onToggle={() => setSidebarOpen((o) => !o)}
          sessions={sessions}
          activeSessionId={activeSessionId}
          activeView={activeView}
          ui={ui}
          onSelectSession={(id) => {
            setDraftPolicy(null)
            setLiveTurnError(null)
            setStoppedRun(null)
            setActiveView('chat')
            activeSidRef.current = id
            setActiveSessionId(id)
            setEvents([])
            setSiblings({})
            setPendingApprovals([])
            setPendingClarifications([])
            setQueuedPrompts([])
            setSessionTasks([])
            setSessionCheckpoints([])
            setLiveThinking('')
            setLiveText('')
            setLiveTools([])
            setLiveOrchestrator(null)
            setIsRunning(false)
            setTurnStartedAt(null)
            setBranchParent(undefined)
            loadSession(id)
          }}
          onNewSession={() => {
            loadSessionSeqRef.current += 1
            loadSessionAbortRef.current?.abort()
            setDraftPolicy(null)
            setLiveTurnError(null)
            setStoppedRun(null)
            activeSidRef.current = null
            setActiveSessionId(null)
            setEvents([])
            setSiblings({})
            setPendingApprovals([])
            setPendingClarifications([])
            setQueuedPrompts([])
            setSessionTasks([])
            setSessionCheckpoints([])
            setLiveThinking('')
            setLiveText('')
            setLiveTools([])
            setLiveOrchestrator(null)
            setIsRunning(false)
            setTurnStartedAt(null)
            setBranchParent(undefined)
            setActiveView('chat')
          }}
          onClearActiveSession={() => {
            if (!activeSessionId) return
            setClearConfirmOpen(true)
          }}
          onDeleteSession={async (id) => {
            try {
              await api(`/api/sessions/${encodeURIComponent(id)}`, { method: 'DELETE' })
              const remaining = sessions.filter((s) => s.id !== id)
              setSessions(remaining)
              if (activeSessionId === id) {
                const nextId = remaining[0]?.id || null
                activeSidRef.current = nextId
                setActiveSessionId(nextId)
                if (nextId) loadSession(nextId)
                else setEvents([])
              }
            } catch (e: any) {
              toast(e?.message || 'Failed to delete session', 'error')
            }
          }}
          onOpenSettings={() => setActiveView('setup')}
        />
      )}

      {/* Main Open Canvas — ZERO topbar chrome! */}
      <main className="relative flex min-w-0 flex-1 flex-col overflow-hidden">
        {activeView === 'chat' && (
          <div
            data-chat-bg={ui.components.chat_page.background}
            className="zenko-chat-bg flex min-h-0 flex-1 flex-col overflow-hidden"
          >
            {activeSession?.parent_session_id && (
              <div className="flex flex-wrap items-center justify-between gap-2 border-b border-[var(--border)] bg-[var(--surface)] px-5 py-2 font-mono text-[11.5px]">
                <div className="flex items-center gap-2 text-[var(--muted)]">
                  <button
                    type="button"
                    onClick={() => {
                      const pid = activeSession.parent_session_id
                      if (!pid) return
                      setActiveSessionId(pid)
                      loadSession(pid)
                    }}
                    className="z-btn z-btn-ghost z-btn-sm font-mono text-[11px]"
                  >
                    ← Parent Session
                  </button>
                  <span className="rounded border border-[var(--border)] bg-[var(--bg)] px-1.5 py-px text-[10px] uppercase text-[var(--fg)]">
                    {activeSession.agent_role || 'subagent'}
                  </span>
                  <span>depth {activeSession.agent_depth ?? 1}</span>
                  {activeSession.worktree_branch && (
                    <span className="text-[var(--accent)]">{activeSession.worktree_branch}</span>
                  )}
                </div>
                <button
                  type="button"
                  onClick={() => setActiveView('telemetry')}
                  className="z-btn z-btn-ghost z-btn-sm font-mono text-[11px]"
                >
                  Subagents Panel
                </button>
              </div>
            )}
            <Phase6Bar
              permissionMode={activePolicy}
              onApprovePlan={async (targetMode, executePrompt) => {
                if (!activeSessionId) return
                try {
                  const res = await api<{ session: Session; task_count: number }>(
                    `/api/sessions/${encodeURIComponent(activeSessionId)}/approve-plan`,
                    {
                      method: 'POST',
                      body: JSON.stringify({
                        mode: targetMode,
                        execute_prompt: executePrompt || null,
                      }),
                    },
                  )
                  setSessions((prev) =>
                    prev.map((s) => (s.id === activeSessionId ? res.session : s)),
                  )
                } catch (e: any) {
                  toast(e.message, 'error')
                }
              }}
            />
            <ChatStream
              sessionId={activeSessionId}
              events={events}
              siblings={siblings}
              pendingApprovals={pendingApprovals}
              pendingClarifications={pendingClarifications}
              liveOrchestrator={liveOrchestrator}
              liveThinking={liveThinking}
              liveText={liveText}
              liveTools={liveTools}
              isRunning={isRunning}
              turnStartedAt={turnStartedAt}
              liveTurnError={liveTurnError}
              stoppedRun={stoppedRun}
              activeModel={activeModel}
              ui={ui}
              setupCompleted={Boolean(config?.general?.setup_completed || hasProviderKey)}
              onQuickPrompt={(p) => sendMessage(p)}
              onRetryTurn={() => {
                const retryText =
                  lastPromptRef.current ||
                  [...events]
                    .reverse()
                    .find((e) => e.role === 'user' && e.content_json?.text)?.content_json?.text ||
                  'Please retry the previous step.'
                setLiveTurnError((prev) => (prev ? { ...prev, retrying: true } : null))
                sendMessage(String(retryText))
              }}
              onContinueStopped={() => {
                setStoppedRun(null)
                sendMessage('Continue from where you stopped.')
              }}
              onDiscardStopped={() => setStoppedRun(null)}
              onRegenerate={(promptText, parentId) => sendMessage(promptText, parentId)}
              onRetryTool={(toolName, argsPreview) =>
                sendMessage(
                  `Please retry tool ${toolName}${argsPreview ? ` with arguments ${argsPreview}` : ''}.`,
                )
              }
              onOpenSetup={() => setActiveView('setup')}
              onEditBranch={(parentId, text) => {
                setBranchParent(parentId)
                setComposerText(text)
              }}
              onSwitchBranch={async (eventId) => {
                if (!activeSessionId) return
                try {
                  await api(`/api/sessions/${encodeURIComponent(activeSessionId)}/branch`, {
                    method: 'POST',
                    body: JSON.stringify({ event_id: eventId }),
                  })
                  loadSession(activeSessionId)
                } catch (e: any) {
                  toast(e?.message || 'Failed to switch branch', 'error')
                }
              }}
              onResolveApproval={async (id, decision) => {
                try {
                  await api(`/api/approvals/${encodeURIComponent(id)}`, {
                    method: 'POST',
                    body: JSON.stringify({ decision }),
                  })
                  setPendingApprovals((prev) => prev.filter((a) => a.id !== id))
                } catch (e: any) {
                  toast(e?.message || 'Failed to resolve approval', 'error')
                }
              }}
              onResolveClarification={async (id, selectedOptionId, customAnswer) => {
                try {
                  await api(`/api/clarifications/${encodeURIComponent(id)}`, {
                    method: 'POST',
                    body: JSON.stringify({
                      selected_option_id: selectedOptionId,
                      custom_answer: customAnswer || null,
                    }),
                  })
                  setPendingClarifications((prev) => prev.filter((c) => c.id !== id))
                } catch (e: any) {
                  toast(e?.message || 'Failed to resolve clarification', 'error')
                }
              }}
            />

            <PromptComposer
              value={composerText}
              onChange={setComposerText}
              onSend={() => sendMessage()}
              onStop={async () => {
                if (!activeSessionId) return
                const partial = liveTextRef.current.trim()
                try {
                  await api(`/api/sessions/${encodeURIComponent(activeSessionId)}/stop`, { method: 'POST' })
                  setIsRunning(false)
                  setLiveThinking('')
                  setLiveText('')
                  setLiveTools([])
                  setStoppedRun({
                    words: partial ? partial.split(' ').filter(Boolean).slice(0, 60) : [],
                    reason: 'Stopped by user',
                  })
                } catch (e: any) {
                  toast(e?.message || 'Failed to stop active turn', 'error')
                }
              }}
              queuedPrompts={queuedPrompts}
              onCancelQueued={async (qid) => {
                if (!activeSessionId) return
                try {
                  await api(
                    `/api/sessions/${encodeURIComponent(activeSessionId)}/queue/${encodeURIComponent(qid)}`,
                    { method: 'DELETE' },
                  )
                  setQueuedPrompts((prev) => prev.filter((q) => q.id !== qid))
                } catch (e: any) {
                  toast(e.message, 'error')
                }
              }}
              onPromoteQueued={async (qid) => {
                if (!activeSessionId) return
                try {
                  const res = await api<{ steered: boolean }>(
                    `/api/sessions/${encodeURIComponent(activeSessionId)}/queue/${encodeURIComponent(qid)}/steer`,
                    { method: 'POST' },
                  )
                  setQueuedPrompts((prev) => prev.filter((q) => q.id !== qid))
                  if (res.steered) toast('Promoted queued prompt to live steer', 'info')
                } catch (e: any) {
                  toast(e.message, 'error')
                }
              }}
              isRunning={isRunning}
              isOrchestratingBg={isOrchestratingBg}
              activeProvider={activeProvider}
              activeModel={activeModel}
              providers={providers}
              configuredProviders={configuredProviders}
              models={models}
              onSelectProviderModel={async (providerId, modelId) => {
                try {
                  let sid = activeSessionId
                  if (!sid) {
                    const created = await api<Session>('/api/sessions', {
                      method: 'POST',
                      body: JSON.stringify({ title: 'New Session' }),
                    })
                    setSessions((prev) => [created, ...prev])
                    activeSidRef.current = created.id
                    setActiveSessionId(created.id)
                    sid = created.id
                  }
                  const res = await api<{ ok: boolean; config?: any }>(
                    `/api/sessions/${encodeURIComponent(sid)}`,
                    {
                      method: 'PATCH',
                      body: JSON.stringify({ provider_kind: providerId, model_override: modelId }),
                    },
                  )
                  if (res.config) setConfig(res.config)
                  setSessions((prev) =>
                    prev.map((s) => (s.id === sid ? { ...s, model_override: modelId } : s)),
                  )
                  api<ModelInfo[]>(`/api/models?provider=${encodeURIComponent(providerId)}`)
                    .then(setModels)
                    .catch(() => {})
                  const label = providers.find((p) => p.id === providerId)?.label || providerId
                  toast(`Active: ${label} · ${modelId}`, 'success')
                } catch (e: any) {
                  toast(e?.message || 'Failed to switch model', 'error')
                }
              }}
              permissionMode={activePolicy}
              onChangePermission={async (p) => {
                if (!activeSessionId) {
                  setDraftPolicy(p)
                  toast(`Execution policy: ${p}`, 'info')
                  return
                }
                try {
                  await api(`/api/sessions/${encodeURIComponent(activeSessionId)}`, {
                    method: 'PATCH',
                    body: JSON.stringify({ permission_mode: p }),
                  })
                  setSessions((prev) =>
                    prev.map((s) => (s.id === activeSessionId ? { ...s, permission_mode: p } : s)),
                  )
                } catch (e: any) {
                  toast(e?.message || 'Failed to update permission mode', 'error')
                }
              }}
              streamingEnabled={streamingEnabled}
              onToggleStreaming={async () => {
                try {
                  let targetSid = activeSessionId || sessions[0]?.id || null
                  if (!targetSid) {
                    const created = await api<Session>('/api/sessions', {
                      method: 'POST',
                      body: JSON.stringify({ title: 'New Session' }),
                    })
                    setSessions((prev) => [created, ...prev])
                    activeSidRef.current = created.id
                    setActiveSessionId(created.id)
                    targetSid = created.id
                  }
                  const next = !streamingEnabled
                  const res = await api<{ ok: boolean; config?: any }>(
                    `/api/sessions/${encodeURIComponent(targetSid)}`,
                    {
                      method: 'PATCH',
                      body: JSON.stringify({ enable_streaming: next }),
                    },
                  )
                  if (res.config) setConfig(res.config)
                  toast(`Streaming ${next ? 'ON' : 'OFF'}`, 'info')
                } catch (e: any) {
                  toast(e?.message || 'Failed to toggle streaming', 'error')
                }
              }}
              activeSessionId={activeSessionId}
              totalTokens={activeSession?.total_tokens ?? 0}
              branchEditBanner={branchParent !== undefined}
              onCancelBranchEdit={() => setBranchParent(undefined)}
              onOpenSettings={() => setActiveView('setup')}
              composerStyle={ui.components.composer}
              popoverStyle={ui.surface.popover_style || 'gooey'}
              extraTools={
                <WorkspaceToolsPopovers
                  sessionId={activeSessionId}
                  tasks={sessionTasks}
                  checkpoints={sessionCheckpoints}
                  popoverStyle={ui.surface.popover_style || 'gooey'}
                  onRewind={async (checkpointId, mode, overwriteManual) => {
                    if (!activeSessionId) return
                    try {
                      const rep = await api<RewindReport>(
                        `/api/sessions/${encodeURIComponent(activeSessionId)}/rewind`,
                        {
                          method: 'POST',
                          body: JSON.stringify({
                            checkpoint_id: checkpointId,
                            mode,
                            overwrite_manual_edits: overwriteManual,
                            summarize_abandoned: true,
                          }),
                        },
                      )
                      await loadSession(activeSessionId)
                      toast(
                        `Rewound (${rep.restored_files.length} restored, ${rep.deleted_files.length} removed${
                          rep.preserved_manual_edits.length > 0
                            ? `, ${rep.preserved_manual_edits.length} hand-edit kept`
                            : ''
                        })`,
                        'success',
                      )
                    } catch (e: any) {
                      toast(e.message, 'error')
                    }
                  }}
                />
              }
            />
          </div>
        )}

        <Suspense
          fallback={
            <div className="flex flex-1 items-center justify-center text-xs text-[var(--muted)]">
              Loading view...
            </div>
          }
        >
        {activeView === 'appearance' && (
          <AppearanceStudio
            ui={ui}
            presets={presets}
            studioSchema={studioSchema}
            providers={providers}
            configuredProviders={configuredProviders}
            models={models}
            activeProvider={activeProvider}
            activeModel={activeModel}
            initialTarget={studioJumpTarget}
            onConsumeInitialTarget={() => setStudioJumpTarget(null)}
            onPatchUi={patchUi}
            onSelectPreset={selectPreset}
            onTestToast={toast}
            onBackToChat={() => setActiveView('chat')}
          />
        )}

        {activeView === 'gateways' && (
          <GatewaysView
            toast={toast}
            tabsVariant={ui.surface.tabs_variant || 'pill'}
            smoothScroll={ui.motion.smooth_scroll !== false}
          />
        )}
        {activeView === 'memory' && (
          <MemoryView
            toast={toast}
            tabsVariant={ui.surface.tabs_variant || 'pill'}
            smoothScroll={ui.motion.smooth_scroll !== false}
          />
        )}
        {activeView === 'automation' && (
          <AutomationView
            toast={toast}
            hubDir={hubDir}
            hubCategories={hubCategories}
            tabsVariant={ui.surface.tabs_variant || 'pill'}
            smoothScroll={ui.motion.smooth_scroll !== false}
          />
        )}
        {activeView === 'telemetry' && (
          <DashboardView
            ui={ui}
            telemetry={telemetry}
            logs={logs}
            jobs={jobs}
            subagents={subagents}
            subagentRoles={subagentRoles}
            taskKinds={taskKinds}
            logCategories={logCategories}
            logCounts={logCounts}
            logsDir={logsDir}
            onOpenCustomizeDashboard={() => {
              setStudioJumpTarget({ categoryId: 'dashboard', subId: 'dashboard_charts' })
              setActiveView('appearance')
            }}
            onKillAgent={async (agentId) => {
              try {
                await api(`/api/agents/${encodeURIComponent(agentId)}`, { method: 'DELETE' })
                const res = await api<{ agents: SubagentRecord[] }>('/api/agents')
                setSubagents(res.agents || [])
                toast('Subagent terminated', 'info')
              } catch (e: any) {
                toast(e.message, 'error')
              }
            }}
            onMergeAgent={async (agentId) => {
              try {
                const rep = await api<{ branch: string; merged: boolean }>(
                  `/api/agents/${encodeURIComponent(agentId)}/merge`,
                  { method: 'POST', body: JSON.stringify({ cleanup: true }) },
                )
                const res = await api<{ agents: SubagentRecord[] }>('/api/agents')
                setSubagents(res.agents || [])
                toast(
                  rep.merged
                    ? `Merged ${rep.branch} cleanly`
                    : `No changes to merge from ${rep.branch}`,
                  'success',
                )
              } catch (e: any) {
                toast(e.message, 'error')
              }
            }}
            onDiscardAgent={async (agentId) => {
              try {
                const rep = await api<{ branch: string }>(
                  `/api/agents/${encodeURIComponent(agentId)}/discard`,
                  { method: 'POST' },
                )
                const res = await api<{ agents: SubagentRecord[] }>('/api/agents')
                setSubagents(res.agents || [])
                toast(`Discarded worktree ${rep.branch}`, 'info')
              } catch (e: any) {
                toast(e.message, 'error')
              }
            }}
            onPruneWorktrees={async () => {
              try {
                const rep = await api<{ removed_count: number }>('/api/agents/prune', {
                  method: 'POST',
                })
                toast(`Pruned ${rep.removed_count} orphan worktree(s)`, 'info')
              } catch (e: any) {
                toast(e.message, 'error')
              }
            }}
            onMessageAgent={async (agentId, message, interrupt) => {
              try {
                const mail = await api<{ delivery_action?: string }>(
                  `/api/agents/${encodeURIComponent(agentId)}/message`,
                  {
                    method: 'POST',
                    body: JSON.stringify({ message, resume: true, interrupt }),
                  },
                )
                const res = await api<{ agents: SubagentRecord[] }>('/api/agents')
                setSubagents(res.agents || [])
                toast(`Delivered to subagent (${mail.delivery_action || 'queued'})`, 'success')
              } catch (e: any) {
                toast(e.message, 'error')
              }
            }}
            onInspectAgent={(agentId) => {
              setActiveView('chat')
              setActiveSessionId(agentId)
              loadSession(agentId)
            }}
            onStopJob={async (jobId) => {
              try {
                await api(`/api/jobs/${encodeURIComponent(jobId)}/stop`, { method: 'POST' })
                api<BackgroundJob[]>('/api/jobs').then(setJobs).catch(() => {})
              } catch (e: any) {
                toast(e.message, 'error')
              }
            }}
            onRestartJob={async (jobId) => {
              try {
                await api(`/api/jobs/${encodeURIComponent(jobId)}/restart`, { method: 'POST' })
                api<BackgroundJob[]>('/api/jobs').then(setJobs).catch(() => {})
                toast('Task restarted', 'success')
              } catch (e: any) {
                toast(e.message, 'error')
              }
            }}
            onDeleteJob={async (jobId) => {
              try {
                await api(`/api/jobs/${encodeURIComponent(jobId)}`, { method: 'DELETE' })
                setJobs((prev) => prev.filter((j) => j.id !== jobId))
              } catch (e: any) {
                toast(e.message, 'error')
              }
            }}
            onPruneJobs={async (kind) => {
              try {
                const res = await api<{ pruned: number }>('/api/jobs/prune', {
                  method: 'POST',
                  body: JSON.stringify({ kind: kind === 'all' ? null : kind }),
                })
                api<BackgroundJob[]>('/api/jobs').then(setJobs).catch(() => {})
                toast(`Pruned ${res.pruned} finished task(s)`, 'info')
              } catch (e: any) {
                toast(e.message, 'error')
              }
            }}
            onQueryLogs={async (params) => {
              const qs = new URLSearchParams()
              if (params.category && params.category !== 'all') qs.set('category', params.category)
              if (params.level && params.level !== 'all') qs.set('level', params.level)
              if (params.sessionId) qs.set('session_id', params.sessionId)
              if (params.search) qs.set('search', params.search)
              qs.set('limit', '200')
              try {
                const rows = await api<LogEntry[]>(`/api/logs?${qs.toString()}`)
                setLogs(rows)
              } catch {
                /* keep existing buffer on transient error */
              }
            }}
            onRefresh={() => {
              api<DaemonTelemetry>('/api/telemetry').then(setTelemetry).catch(() => {})
              api<LogEntry[]>('/api/logs?limit=200').then(setLogs).catch(() => {})
              api<BackgroundJob[]>('/api/jobs').then(setJobs).catch(() => {})
              api<{ agents: SubagentRecord[] }>('/api/agents')
                .then((res) => setSubagents(res.agents || []))
                .catch(() => {})
            }}
          />
        )}
        {activeView === 'setup' && (
          <SetupView
            config={config}
            providers={providers}
            configuredProviders={configuredProviders}
            hasTelegram={hasTelegram}
            webSearchProviders={webSearchProviders}
            hasBraveSearchKey={hasBraveSearchKey}
            hasTavilySearchKey={hasTavilySearchKey}
            mcpServers={mcpServers}
            lspRegistry={lspRegistry}
            lspActive={lspActive}
            onMcpUpdated={setMcpServers}
            onLspUpdated={setLspActive}
            onSaved={() => {
              bootstrap()
              setActiveView('chat')
            }}
            onRefreshBootstrap={() => {
              bootstrap()
            }}
            tabsVariant={ui.surface.tabs_variant || 'pill'}
            smoothScroll={ui.motion.smooth_scroll !== false}
            toast={toast}
          />
        )}
        </Suspense>
      </main>

      {/* Signature Floating Right Vertical Icon Dock — hidden inside full-page Theme Studio */}
      {activeView !== 'appearance' && (
        <FloatingDock
          activeView={activeView}
          onSelectView={setActiveView}
          ui={ui}
          onToggleMode={() =>
            selectPreset(ui.preset_id || 'zenko_vermilion', ui.mode === 'light' ? 'dark' : 'light')
          }
          onOpenQuickCustomizer={() => setQuickCustomizerOpen((o) => !o)}
          monitorOpen={monitorOpen}
          activeBgCount={activeBgCount}
          onToggleMonitor={() => setMonitorOpen((o) => !o)}
          collapsed={dockCollapsed}
          onToggleCollapse={() => setDockCollapsed((c) => !c)}
        />
      )}

      {/* Compact Draggable, Hideable Live Terminal & Background Tasks Monitor */}
      {activeView !== 'appearance' && (
        <FloatingActivityMonitor
          open={monitorOpen}
          onClose={() => setMonitorOpen(false)}
          jobs={jobs}
          subagents={subagents}
          terminalBuffers={terminalBuffers}
          onHydrateBuffer={(jobId, content) =>
            setTerminalBuffers((prev) => ({ ...prev, [jobId]: content }))
          }
          activeSessionId={activeSessionId}
          isSessionRunning={isRunning}
          popoverStyle={ui.surface.popover_style || 'gooey'}
          tabsVariant={ui.surface.tabs_variant || 'pill'}
          onStopJob={async (jobId) => {
            try {
              await api(`/api/jobs/${encodeURIComponent(jobId)}/stop`, { method: 'POST' })
              toast('Task stopped', 'info')
            } catch (e: any) {
              toast(e.message, 'error')
            }
          }}
          onClearJobOutput={async (jobId) => {
            try {
              await api(`/api/jobs/${encodeURIComponent(jobId)}/clear`, { method: 'POST' })
              setTerminalBuffers((prev) => ({ ...prev, [jobId]: '' }))
              setJobs((prev) =>
                prev.map((j) => (j.id === jobId ? { ...j, bytes_written: 0 } : j)),
              )
              toast('Task log cleared', 'info')
            } catch (e: any) {
              toast(e.message, 'error')
            }
          }}
          onDeleteJob={async (jobId) => {
            try {
              await api(`/api/jobs/${encodeURIComponent(jobId)}`, { method: 'DELETE' })
              setJobs((prev) => prev.filter((j) => j.id !== jobId))
              setTerminalBuffers((prev) => {
                const next = { ...prev }
                delete next[jobId]
                return next
              })
            } catch (e: any) {
              toast(e.message, 'error')
            }
          }}
          onClearDoneJobs={async () => {
            try {
              const res = await api<{ pruned: number }>('/api/jobs/prune', {
                method: 'POST',
                body: JSON.stringify({ kind: null }),
              })
              const refreshed = await api<BackgroundJob[]>('/api/jobs').catch(() => [])
              setJobs(refreshed)
              setTerminalBuffers({})
              toast(
                res.pruned > 0
                  ? `Cleared ${res.pruned} finished task log(s)`
                  : 'Cleared zenko_task buffers',
                'info',
              )
            } catch (e: any) {
              toast(e.message, 'error')
            }
          }}
          onSendJobInput={async (jobId, input) => {
            try {
              await api(`/api/jobs/${encodeURIComponent(jobId)}/input`, {
                method: 'POST',
                body: JSON.stringify({ input }),
              })
            } catch (e: any) {
              toast(e.message, 'error')
            }
          }}
          onDemoteForeground={async () => {
            if (!activeSessionId) return
            try {
              const res = await api<{ demoted: boolean }>(
                `/api/sessions/${encodeURIComponent(activeSessionId)}/background`,
                { method: 'POST' },
              )
              toast(
                res.demoted
                  ? 'Handed off command to async background job'
                  : 'Command is already running in background',
                'info',
              )
            } catch (e: any) {
              toast(e.message, 'error')
            }
          }}
          onKillAgent={async (agentId) => {
            try {
              await api(`/api/agents/${encodeURIComponent(agentId)}`, { method: 'DELETE' })
              const res = await api<{ agents: SubagentRecord[] }>('/api/agents')
              setSubagents(res.agents || [])
              toast('Subagent terminated', 'info')
            } catch (e: any) {
              toast(e.message, 'error')
            }
          }}
          onInspectAgent={(agentId) => {
            setActiveView('chat')
            setActiveSessionId(agentId)
            loadSession(agentId)
          }}
        />
      )}

      {/* Slide-Over Quick Customizer Sheet (Cmd/Ctrl + .) */}
      <AnimatePresence>
        {quickCustomizerOpen && (
          <div
            className="fixed inset-0 z-50 flex justify-end bg-black/55 backdrop-blur-xs"
            onClick={(e) => e.target === e.currentTarget && setQuickCustomizerOpen(false)}
          >
            <motion.div
              initial={{ x: 380 }}
              animate={{ x: 0 }}
              exit={{ x: 380 }}
              transition={SPRING_PANEL}
              className="flex h-full w-full max-w-[380px] flex-col border-l border-[var(--border)] bg-[var(--surface)]"
            >
              <CompactScrollArea className="min-h-0 flex-1" viewportClassName="p-5 pr-6">
                <div className="flex flex-col gap-5">
                  <div className="flex items-center justify-between">
                    <strong className="text-[15px]">Color Themes</strong>
                    <div className="flex items-center gap-2">
                      <ThemeToggle
                        mode={ui.mode === 'light' ? 'light' : 'dark'}
                        variant={ui.motion.theme_transition || 'circle-blur'}
                        start="top-right"
                        onToggle={() =>
                          selectPreset(ui.preset_id || 'zenko_vermilion', ui.mode === 'light' ? 'dark' : 'light')
                        }
                        className="h-7 w-7 rounded-lg border border-[var(--border)] bg-[var(--bg)] text-[var(--fg)] hover:border-[var(--accent)]/50"
                        iconClassName="size-3.5"
                      />
                      <button
                        type="button"
                        onClick={() => setQuickCustomizerOpen(false)}
                        className="cursor-pointer text-[var(--muted)] hover:text-[var(--fg)]"
                      >
                        <X size={16} />
                      </button>
                    </div>
                  </div>
                  <ThemeSelector
                    className="grid-cols-2 sm:grid-cols-2 lg:grid-cols-2"
                    value={ui.preset_id}
                    mode={ui.mode === 'light' ? 'light' : 'dark'}
                    onChange={selectPreset}
                    themes={presets}
                  />
                  <button
                    type="button"
                    onClick={() => {
                      setQuickCustomizerOpen(false)
                      setActiveView('appearance')
                    }}
                    className="z-btn z-btn-primary w-full"
                  >
                    Open Customization
                  </button>
                </div>
              </CompactScrollArea>
            </motion.div>
          </div>
        )}
      </AnimatePresence>

      {/* Cmd+K Command Palette — EaseUI CommandPalette | BeUI CenterMorphModal | Classic Scale */}
      {ui.surface.command_palette_style === 'easeui_palette' ? (
        <EaseCommandPalette
          open={cmdKOpen}
          onOpenChange={setCmdKOpen}
          label="Zenko Command Palette"
        >
          <EaseCommandInput placeholder="Jump to view or run action..." />
          <EaseCommandList>
            <EaseCommandEmpty>No matching commands.</EaseCommandEmpty>
            <EaseCommandGroup heading="Views">
              {COMMAND_PALETTE_VIEWS.map(([view, label]) => (
                <EaseCommandItem
                  key={view}
                  value={label}
                  keywords={[view]}
                  onSelect={() => setActiveView(view)}
                >
                  <span className="flex-1">{label}</span>
                  <span className="font-mono text-[11px] text-[var(--muted)]">{view}</span>
                </EaseCommandItem>
              ))}
            </EaseCommandGroup>
            <EaseCommandSeparator />
            <EaseCommandGroup heading="Quick Actions">
              <EaseCommandItem
                value="Toggle Dark / Light Mode"
                keywords={['theme', 'dark', 'light']}
                onSelect={() =>
                  selectPreset(
                    ui.preset_id || 'zenko_vermilion',
                    ui.mode === 'light' ? 'dark' : 'light',
                  )
                }
              >
                <span className="flex-1">Toggle Dark / Light Mode</span>
                <span className="font-mono text-[11px] text-[var(--muted)]">{ui.mode}</span>
              </EaseCommandItem>
              <EaseCommandItem
                value="Quick Theme Customizer"
                keywords={['colors', 'palette', 'appearance']}
                onSelect={() => setQuickCustomizerOpen(true)}
              >
                <span className="flex-1">Quick Theme Customizer</span>
                <span className="font-mono text-[11px] text-[var(--muted)]">⌘.</span>
              </EaseCommandItem>
              <EaseCommandItem
                value="Toggle Activity Monitor"
                keywords={['terminal', 'jobs', 'subagents']}
                onSelect={() => setMonitorOpen((o) => !o)}
              >
                <span className="flex-1">Toggle Activity Monitor</span>
                <span className="font-mono text-[11px] text-[var(--muted)]">
                  {activeBgCount} active
                </span>
              </EaseCommandItem>
            </EaseCommandGroup>
          </EaseCommandList>
        </EaseCommandPalette>
      ) : ui.surface.modal_style !== 'scale' ? (
        <CenterMorphModal open={cmdKOpen} onOpenChange={setCmdKOpen}>
          <CenterMorphModalContent
            ariaLabel="Command Palette"
            showCloseButton={false}
            className="max-w-[500px] rounded-[24px] bg-[var(--surface)]"
          >
            <div className="flex items-center gap-2.5 border-b border-[var(--border)] px-4 py-3">
              <Search size={15} className="text-[var(--muted)]" />
              <input
                autoFocus
                value={cmdQuery}
                onChange={(e) => setCmdQuery(e.target.value)}
                placeholder="Jump to view..."
                className="w-full bg-transparent text-[14px] text-[var(--fg)] focus:outline-none"
              />
            </div>
            <div className="flex flex-col p-2">
              {COMMAND_PALETTE_VIEWS.filter(([, label]) =>
                label.toLowerCase().includes(cmdQuery.toLowerCase()),
              ).map(([view, label]) => (
                <button
                  key={view}
                  type="button"
                  onClick={() => {
                    setActiveView(view)
                    setCmdKOpen(false)
                    setCmdQuery('')
                  }}
                  className="flex cursor-pointer items-center justify-between rounded-lg px-3 py-2 text-left text-[13px] text-[var(--fg)] hover:bg-[var(--card)]"
                >
                  <span>{label}</span>
                  <span className="font-mono text-[11px] text-[var(--muted)]">{view}</span>
                </button>
              ))}
            </div>
          </CenterMorphModalContent>
        </CenterMorphModal>
      ) : (
        <AnimatePresence>
          {cmdKOpen && (
            <div
              className="fixed inset-0 z-50 flex items-start justify-center bg-black/60 pt-[14vh] backdrop-blur-xs"
              onClick={(e) => e.target === e.currentTarget && setCmdKOpen(false)}
            >
              <motion.div
                initial={{ opacity: 0, scale: 0.96, y: -8 }}
                animate={{ opacity: 1, scale: 1, y: 0 }}
                exit={{ opacity: 0, scale: 0.96, y: -8 }}
                transition={SPRING_PANEL}
                className="w-full max-w-[500px] overflow-hidden rounded-2xl border border-[var(--border)] bg-[var(--surface)] shadow-2xl"
              >
                <div className="flex items-center gap-2.5 border-b border-[var(--border)] px-4 py-3">
                  <Search size={15} className="text-[var(--muted)]" />
                  <input
                    autoFocus
                    value={cmdQuery}
                    onChange={(e) => setCmdQuery(e.target.value)}
                    placeholder="Jump to view..."
                    className="w-full bg-transparent text-[14px] focus:outline-none"
                  />
                </div>
                <div className="flex flex-col p-2">
                  {COMMAND_PALETTE_VIEWS.filter(([, label]) =>
                    label.toLowerCase().includes(cmdQuery.toLowerCase()),
                  ).map(([view, label]) => (
                    <button
                      key={view}
                      type="button"
                      onClick={() => {
                        setActiveView(view)
                        setCmdKOpen(false)
                        setCmdQuery('')
                      }}
                      className="flex cursor-pointer items-center justify-between rounded-lg px-3 py-2 text-left text-[13px] text-[var(--fg)] hover:bg-[var(--card)]"
                    >
                      <span>{label}</span>
                      <span className="font-mono text-[11px] text-[var(--muted)]">{view}</span>
                    </button>
                  ))}
                </div>
              </motion.div>
            </div>
          )}
        </AnimatePresence>
      )}

      {/* Clear Active Session Confirmation Modal via BeUI CenterMorphModal */}
      <CenterMorphModal open={clearConfirmOpen} onOpenChange={setClearConfirmOpen}>
        <CenterMorphModalContent ariaLabel="Clear Active Session" className="max-w-[400px] p-6">
          <h3 className="text-[16px] font-semibold text-[var(--fg)]">Clear active session?</h3>
          <p className="mt-2 text-[13px] leading-relaxed text-[var(--muted)]">
            This clears messages from the current conversation view while keeping your workspace files and saved memory intact.
          </p>
          <div className="mt-5 flex items-center justify-end gap-2">
            <CenterMorphModalClose>
              <button type="button" className="z-btn z-btn-secondary z-btn-sm">
                Cancel
              </button>
            </CenterMorphModalClose>
            <button
              type="button"
              onClick={async () => {
                setClearConfirmOpen(false)
                if (!activeSessionId) return
                try {
                  await api(`/api/sessions/${encodeURIComponent(activeSessionId)}/clear`, { method: 'POST' })
                  setEvents([])
                  setLiveText('')
                  setLiveThinking('')
                  setLiveTools([])
                  toast('Session cleared', 'success')
                } catch (e: any) {
                  toast(e?.message || 'Failed to clear session', 'error')
                }
              }}
              className="z-btn z-btn-danger z-btn-sm"
            >
              Clear Session
            </button>
          </div>
        </CenterMorphModalContent>
      </CenterMorphModal>

      {/* Toast Stack powered by EaseUI Swipe Toaster or customizable Zenko Toasts (default: Top Center) */}
      {ui.surface.toast_style === 'easeui_toast' ? (
        <EaseToaster position={ui.surface.toast_position || 'top_center'} />
      ) : (
        (() => {
          const toastPos = ui.surface.toast_position || 'top_center'
          const toastStyle = ui.surface.toast_style || 'default'
          const isTop = toastPos.startsWith('top_')
          const posClass = TOAST_POSITION_STACK_CLASS[toastPos] || TOAST_POSITION_STACK_CLASS.top_center
          const yOffset = isTop ? -10 : 10
          return (
            <div
              className={cn(
                'pointer-events-none fixed z-[400] flex w-80 max-w-[92vw] flex-col gap-2',
                posClass,
              )}
            >
              <AnimatePresence>
                {toasts.map((t) => {
                  const accentColor =
                    t.kind === 'error'
                      ? 'var(--destructive)'
                      : t.kind === 'success'
                        ? 'var(--success)'
                        : 'var(--accent)'

                  if (toastStyle === 'minimal_pill') {
                    return (
                      <motion.div
                        key={t.id}
                        initial={{ opacity: 0, y: yOffset, scale: 0.96 }}
                        animate={{ opacity: 1, y: 0, scale: 1 }}
                        exit={{ opacity: 0, y: yOffset, scale: 0.96 }}
                        className="pointer-events-auto inline-flex max-w-full items-center gap-2 rounded-full border border-[var(--border)] bg-[var(--surface)]/95 px-4 py-2 text-[12.5px] text-[var(--fg)] shadow-xl backdrop-blur-md"
                      >
                        <span
                          aria-hidden="true"
                          className="h-2 w-2 shrink-0 rounded-full"
                          style={{ backgroundColor: accentColor }}
                        />
                        <span className="truncate">{t.message}</span>
                      </motion.div>
                    )
                  }

                  if (toastStyle === 'accent_bar') {
                    return (
                      <motion.div
                        key={t.id}
                        initial={{ opacity: 0, y: yOffset }}
                        animate={{ opacity: 1, y: 0 }}
                        exit={{ opacity: 0, y: yOffset }}
                        className="pointer-events-auto w-full rounded-[var(--radius)] border border-[var(--border)] border-l-[3px] bg-[var(--surface)]/96 px-3.5 py-2.5 text-[12.5px] text-[var(--fg)] shadow-xl backdrop-blur-md"
                        style={{ borderLeftColor: accentColor }}
                      >
                        {t.message}
                      </motion.div>
                    )
                  }

                  if (toastStyle === 'glass_toast') {
                    return (
                      <motion.div
                        key={t.id}
                        initial={{ opacity: 0, y: yOffset, scale: 0.97 }}
                        animate={{ opacity: 1, y: 0, scale: 1 }}
                        exit={{ opacity: 0, y: yOffset, scale: 0.97 }}
                        className="pointer-events-auto flex w-full items-center gap-2.5 rounded-[var(--radius-lg)] border bg-[var(--surface)]/70 px-4 py-2.5 text-[12.5px] text-[var(--fg)] shadow-[0_14px_36px_rgba(0,0,0,0.34)] backdrop-blur-xl"
                        style={{
                          borderColor: `color-mix(in srgb, ${accentColor} 38%, var(--border))`,
                        }}
                      >
                        <span
                          aria-hidden="true"
                          className="h-2 w-2 shrink-0 rounded-full"
                          style={{ backgroundColor: accentColor }}
                        />
                        <span className="min-w-0 flex-1">{t.message}</span>
                      </motion.div>
                    )
                  }

                  if (ui.surface.alert_style === 'easeui_alert') {
                    return (
                      <EaseAlert
                        key={t.id}
                        variant={
                          t.kind === 'error'
                            ? 'destructive'
                            : t.kind === 'success'
                              ? 'success'
                              : 'neutral'
                        }
                        className="pointer-events-auto w-full bg-[var(--surface)]/95 px-3.5 py-2.5 text-[12.5px] shadow-xl backdrop-blur-md"
                      >
                        <EaseAlertDescription className="mt-0 text-[12.5px] text-[var(--fg)]">
                          {t.message}
                        </EaseAlertDescription>
                      </EaseAlert>
                    )
                  }

                  if (ui.surface.alert_style !== 'compact') {
                    return (
                      <Alert
                        key={t.id}
                        variant={
                          t.kind === 'error'
                            ? 'destructive'
                            : t.kind === 'success'
                              ? 'success'
                              : 'info'
                        }
                        className="pointer-events-auto w-full bg-[var(--surface)]/95 px-3.5 py-2.5 text-[12.5px] shadow-xl backdrop-blur-md"
                      >
                        <AlertIcon />
                        <AlertContent>
                          <AlertDescription className="text-[12.5px] text-[var(--fg)]">
                            {t.message}
                          </AlertDescription>
                        </AlertContent>
                      </Alert>
                    )
                  }

                  return (
                    <motion.div
                      key={t.id}
                      initial={{ opacity: 0, y: yOffset }}
                      animate={{ opacity: 1, y: 0 }}
                      exit={{ opacity: 0, y: yOffset }}
                      className="pointer-events-auto w-full rounded-xl border bg-[var(--surface)] px-3.5 py-2 text-[12.5px] shadow-xl"
                      style={{
                        borderColor:
                          t.kind === 'error'
                            ? 'var(--destructive)'
                            : t.kind === 'success'
                              ? 'color-mix(in srgb, var(--accent) 52%, var(--border))'
                              : 'var(--border)',
                      }}
                    >
                      {t.message}
                    </motion.div>
                  )
                })}
              </AnimatePresence>
            </div>
          )
        })()
      )}
    </div>
    </ZenkoTooltipProvider>
    </ZenkoIconProvider>
  )
}
