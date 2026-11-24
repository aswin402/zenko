import { useEffect, useRef, useState } from 'react'
import {
  AlertTriangle,
  BrainIcon,
  Check,
  Download,
  FileText,
  History,
  Pencil,
  Plus,
  RefreshCw,
  RotateCcw,
  Search,
  Trash2,
  X,
} from 'lucide-react'
import { api } from '@/lib/api'
import type {
  DreamRunRecord,
  DreamStatus,
  MemoryDocSnapshot,
  MemoryEntry,
  MemoryKindMeta,
  ScoredMemoryHit,
  SessionSearchHit,
  SkillVersionSnapshot,
  WorkspaceSnapshot,
} from '@/lib/types'
import { Tabs, TabsList, TabsTrigger } from '@/components/motion/tabs'
import { SmoothCaretInput } from '@/components/skiperui/smooth-caret'
import { ZenkoSelect } from '@/components/ui/ZenkoSelect'
import { ScrollPage, type SecondaryTabsVariant } from './shared'

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
                  <TabsTrigger
                    key={doc}
                    value={doc}
                    icon={<FileText size={12} />}
                    className="font-mono text-[11.5px]"
                  >
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
              <Search size={13} className="pointer-events-none absolute left-2.5 top-2.5 z-10 text-[var(--muted)]" />
              <SmoothCaretInput
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

