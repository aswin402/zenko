import { Children, isValidElement, type ReactNode } from 'react'
import type { DiffLine } from '@/components/assistant-ui/elements/code-diff'
import type { DiffHunk } from '@/components/assistant-ui/elements/reviewable-diff'
import type { FileTreeNode } from '@/components/assistant-ui/elements/file-tree'
import type { WebSearchResult } from '@/components/assistant-ui/elements/web-search'
import type { MemoryChip } from '@/components/assistant-ui/elements/memory-chips'
import type { DataTableColumn, DataTableRow } from '@/components/assistant-ui/elements/data-table'
import type { Segment } from '@/components/assistant-ui/elements/streaming-text'
import type { ApprovalRecord } from '@/lib/types'

/**
 * Pure, deterministic, zero-regex parsers and structural adapters connecting
 * Zenko runtime session events & tool outputs to Assistant-UI Elements.
 */

export interface ParsedDiffData {
  filename: string
  additions: number
  deletions: number
  lines: DiffLine[]
  hunks: DiffHunk[]
}

export interface ParsedShellData {
  command: string
  language: string
  cwd?: string
  stdoutLines: string[]
  stderrLines: string[]
  exitCode: number
}

export interface ParsedWebSearchData {
  query: string
  results: WebSearchResult[]
}

export interface ParsedFileTreeData {
  title: string
  nodes: FileTreeNode[]
  totalAdditions: number
  totalDeletions: number
}

const SHELL_TOOL_SET = new Set([
  'shell_exec',
  'run_shell',
  'bash',
  'exec',
  'terminal',
  'job_spawn',
])

const CODE_TOOL_SET = new Set(['code_exec', 'run_script', 'script_run', 'python_exec', 'eval'])

const SEARCH_TOOL_SET = new Set(['web_search', 'web_fetch', 'search_web'])

const FILE_TREE_TOOL_SET = new Set(['list_dir', 'glob_find', 'find_files', 'workspace_tree'])

const MEMORY_TOOL_SET = new Set([
  'memory_store',
  'memory_search',
  'memory_recall',
  'memory_update',
  'memory_save',
])

const SKILL_TOOL_SET = new Set(['skill_view', 'skill_manage'])

export type SmartToolKind = 'error' | 'web_search' | 'file_tree' | 'diff' | 'code_runner' | 'shell' | 'generic'

/** Declarative tool classifier — zero regex, deterministic set lookup. */
export function classifySmartTool(
  toolName: string,
  hasDiff: boolean,
  isError: boolean,
): SmartToolKind {
  if (isError) return 'error'
  const lower = toolName.toLowerCase()
  if (SEARCH_TOOL_SET.has(lower)) return 'web_search'
  if (hasDiff) return 'diff'
  if (FILE_TREE_TOOL_SET.has(lower)) return 'file_tree'
  if (CODE_TOOL_SET.has(lower)) return 'code_runner'
  if (SHELL_TOOL_SET.has(lower)) return 'shell'
  return 'generic'
}

/** Safe JSON object parse returning null when args preview is truncated or plain text. */
export function parseJsonObjectSafe(raw?: string): Record<string, unknown> | null {
  if (!raw) return null
  const trimmed = raw.trim()
  if (!trimmed.startsWith('{')) return null
  try {
    const parsed = JSON.parse(trimmed)
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : null
  } catch {
    return null
  }
}

/** Extracts a string field from JSON or truncated JSON preview using deterministic index scanning. */
export function extractJsonField(raw: string | undefined, keys: readonly string[]): string {
  if (!raw) return ''
  const parsed = parseJsonObjectSafe(raw)
  if (parsed) {
    for (const k of keys) {
      const v = parsed[k]
      if (typeof v === 'string' && v.trim()) return v.trim()
    }
  }
  for (const k of keys) {
    const needle = `"${k}":`
    const idx = raw.indexOf(needle)
    if (idx === -1) continue
    let pos = idx + needle.length
    while (pos < raw.length && raw[pos] === ' ') pos += 1
    if (raw[pos] !== '"') continue
    pos += 1
    let out = ''
    let escaped = false
    while (pos < raw.length) {
      const ch = raw[pos]
      if (escaped) {
        out += ch === 'n' ? '\n' : ch === 't' ? '\t' : ch
        escaped = false
      } else if (ch === '\\') {
        escaped = true
      } else if (ch === '"') {
        break
      } else {
        out += ch
      }
      pos += 1
    }
    if (out.trim()) return out.trim()
  }
  return ''
}

/** Parses unified diff output into CodeDiff lines AND ReviewableDiff hunks. */
export function parseUnifiedDiff(diff: string, fallbackName = 'patch.diff'): ParsedDiffData {
  const rawLines = diff.split('\n')
  let filename = fallbackName
  let additions = 0
  let deletions = 0
  const lines: DiffLine[] = []
  const hunks: DiffHunk[] = []
  let activeHunk: { id: string; range: string; lines: DiffLine[] } | null = null

  const pushHunk = () => {
    if (activeHunk && activeHunk.lines.length > 0) {
      hunks.push({
        id: activeHunk.id,
        range: activeHunk.range,
        decision: 'pending',
        lines: activeHunk.lines,
      })
    }
    activeHunk = null
  }

  for (const raw of rawLines) {
    if (raw.startsWith('+++ ')) {
      const pathPart = raw.slice(4).trim()
      const cleaned = pathPart.startsWith('b/') ? pathPart.slice(2) : pathPart
      if (cleaned && cleaned !== '/dev/null') filename = cleaned
      continue
    }
    if (raw.startsWith('--- ') || raw.startsWith('diff --git ') || raw.startsWith('index ')) {
      continue
    }
    if (raw.startsWith('@@')) {
      pushHunk()
      const endAt = raw.indexOf('@@', 2)
      const range = endAt > 2 ? raw.slice(0, endAt + 2).trim() : raw.trim()
      activeHunk = { id: `hunk-${hunks.length + 1}`, range, lines: [] }
      continue
    }

    let diffLine: DiffLine
    if (raw.startsWith('+')) {
      additions += 1
      diffLine = { kind: 'added', text: raw.slice(1) }
    } else if (raw.startsWith('-')) {
      deletions += 1
      diffLine = { kind: 'removed', text: raw.slice(1) }
    } else {
      diffLine = { kind: 'context', text: raw.startsWith(' ') ? raw.slice(1) : raw }
    }

    lines.push(diffLine)
    if (!activeHunk) {
      activeHunk = { id: 'hunk-1', range: '@@ changes @@', lines: [] }
    }
    activeHunk.lines.push(diffLine)
  }
  pushHunk()

  return {
    filename,
    additions,
    deletions,
    lines: lines.slice(0, 120),
    hunks: hunks.slice(0, 12),
  }
}

function trimTrailingSlashes(value: string): string {
  let end = value.length
  while (end > 0 && value[end - 1] === '/') end -= 1
  return value.slice(0, end)
}

function trimLeadingListPrefix(value: string): string {
  let i = 0
  while (i < value.length) {
    const ch = value[i]
    const isDigit = ch >= '0' && ch <= '9'
    if (isDigit || ch === '.' || ch === '-' || ch === '*' || ch === '#' || ch === ' ') {
      i += 1
    } else {
      break
    }
  }
  return value.slice(i)
}

/** Parses directory or glob listing text into a sorted hierarchical FileTreeNode array. */
export function parseFileTreeFromTool(
  toolName: string,
  argsPreview: string | undefined,
  content: string,
): ParsedFileTreeData {
  const dirArg = extractJsonField(argsPreview, ['path', 'dir', 'directory', 'pattern'])
  const rawPaths: { path: string; isDir: boolean }[] = []

  const jsonObj = parseJsonObjectSafe(content)
  if (jsonObj && Array.isArray(jsonObj.entries)) {
    for (const entry of jsonObj.entries) {
      if (typeof entry === 'string' && entry.trim()) {
        const clean = entry.trim()
        rawPaths.push({ path: trimTrailingSlashes(clean), isDir: clean.endsWith('/') })
      } else if (entry && typeof entry === 'object') {
        const rec = entry as Record<string, unknown>
        const p = String(rec.path ?? rec.name ?? '').trim()
        if (p) rawPaths.push({ path: trimTrailingSlashes(p), isDir: Boolean(rec.is_dir || rec.kind === 'dir') })
      }
    }
  } else {
    for (const line of content.split('\n')) {
      const trimmed = line.trim()
      if (!trimmed || trimmed.startsWith('(') || trimmed.startsWith('Total')) continue
      // Strip common "[dir] " or "- " list prefixes deterministically
      let item = trimmed
      let isDir = false
      if (item.startsWith('[dir]')) {
        isDir = true
        item = item.slice(5).trim()
      } else if (item.startsWith('[file]')) {
        item = item.slice(6).trim()
      } else if (item.startsWith('- ')) {
        item = item.slice(2).trim()
      }
      if (item.endsWith('/')) {
        isDir = true
        item = item.slice(0, -1)
      }
      if (item.startsWith('./')) item = item.slice(2)
      if (!item || item.includes(' -> ')) continue
      rawPaths.push({ path: item, isDir })
    }
  }

  const nodeMap = new Map<string, FileTreeNode>()
  for (const entry of rawPaths.slice(0, 48)) {
    const segments = entry.path.split('/').filter(Boolean)
    let accum = ''
    for (let i = 0; i < segments.length; i += 1) {
      const seg = segments[i]
      accum = accum ? `${accum}/${seg}` : seg
      const isLast = i === segments.length - 1
      const kind: 'folder' | 'file' = !isLast || entry.isDir ? 'folder' : 'file'
      if (!nodeMap.has(accum)) {
        nodeMap.set(accum, { path: accum, name: seg, depth: i, kind })
      }
    }
  }

  const nodes = Array.from(nodeMap.values())
  const fileCount = nodes.filter((n) => n.kind === 'file').length
  const title = dirArg
    ? `${dirArg} · ${fileCount} ${fileCount === 1 ? 'file' : 'files'}`
    : `${toolName} · ${fileCount} ${fileCount === 1 ? 'file' : 'files'}`

  return { title, nodes, totalAdditions: 0, totalDeletions: 0 }
}

/** Extracts domain safely from an http/https URL using the URL standard parser. */
function extractHost(url: string): string {
  try {
    const parsed = new URL(url)
    return parsed.hostname.startsWith('www.') ? parsed.hostname.slice(4) : parsed.hostname
  } catch {
    return ''
  }
}

/** Deterministic parser for web_search / web_fetch tool results into WebSearch props. */
export function parseWebSearchFromTool(
  argsPreview: string | undefined,
  content: string,
): ParsedWebSearchData {
  const query =
    extractJsonField(argsPreview, ['query', 'q', 'url', 'search']) || 'web search'
  const results: WebSearchResult[] = []
  const seen = new Set<string>()

  const pushResult = (title: string, url: string) => {
    const domain = extractHost(url)
    if (!domain) return
    const key = `${domain}::${url}`
    if (seen.has(key)) return
    seen.add(key)
    results.push({ title: title.trim() || domain, domain, url })
  }

  const parsedJson = parseJsonObjectSafe(content)
  const listField = parsedJson?.results ?? parsedJson?.items ?? parsedJson?.sources
  if (Array.isArray(listField)) {
    for (const item of listField) {
      if (!item || typeof item !== 'object') continue
      const rec = item as Record<string, unknown>
      const url = String(rec.url ?? rec.href ?? rec.link ?? '')
      const title = String(rec.title ?? rec.name ?? '')
      if (url) pushResult(title, url)
    }
  }

  // Also scan markdown lines for `[Title](https://...)` or plain `https://...` URLs (zero regex)
  if (results.length === 0 && content) {
    const lines = content.split('\n')
    for (const line of lines) {
      let cursor = 0
      while (cursor < line.length && results.length < 10) {
        const httpIdx = line.indexOf('http', cursor)
        if (httpIdx === -1) break
        const isHttps = line.startsWith('https://', httpIdx)
        const isHttp = !isHttps && line.startsWith('http://', httpIdx)
        if (!isHttps && !isHttp) {
          cursor = httpIdx + 4
          continue
        }
        let end = httpIdx
        while (end < line.length) {
          const ch = line[end]
          if (ch === ' ' || ch === ')' || ch === ']' || ch === '>' || ch === '"' || ch === '\'' || ch === '\t') {
            break
          }
          end += 1
        }
        const url = line.slice(httpIdx, end)
        let title = ''
        if (httpIdx >= 2 && line[httpIdx - 1] === '(' && line[httpIdx - 2] === ']') {
          const openBracket = line.lastIndexOf('[', httpIdx - 2)
          if (openBracket !== -1) {
            title = line.slice(openBracket + 1, httpIdx - 2)
          }
        }
        if (!title) {
          const before = line.slice(0, httpIdx).trim()
          title = before.length > 3 && before.length < 90 ? trimLeadingListPrefix(before) : ''
        }
        pushResult(title, url)
        cursor = end + 1
      }
    }
  }

  return { query, results }
}

/** Parses shell / code runner tool invocation into TerminalBlock & CodeRunner props. */
export function parseShellOrCodeTool(
  toolName: string,
  argsPreview: string | undefined,
  content: string,
  isError: boolean,
): ParsedShellData {
  const command =
    extractJsonField(argsPreview, ['command', 'cmd', 'code', 'script']) ||
    argsPreview?.trim() ||
    toolName
  const cwd = extractJsonField(argsPreview, ['cwd', 'workdir', 'working_dir']) || undefined
  const language =
    extractJsonField(argsPreview, ['language', 'lang']) ||
    (toolName.toLowerCase().includes('python') ? 'python' : 'bash')

  const rawLines = content ? content.split('\n') : []
  let exitCode = isError ? 1 : 0
  const stdoutLines: string[] = []
  const stderrLines: string[] = []
  let inStderr = false

  for (const line of rawLines) {
    if (line.startsWith('exit_code:') || line.startsWith('Exit code:')) {
      const parsedNum = Number(line.slice(line.indexOf(':') + 1).trim())
      if (Number.isFinite(parsedNum)) exitCode = parsedNum
      continue
    }
    if (line.trim() === '--- stderr ---' || line.trim() === 'stderr:') {
      inStderr = true
      continue
    }
    if (line.trim() === '--- stdout ---' || line.trim() === 'stdout:') {
      inStderr = false
      continue
    }
    if (inStderr) stderrLines.push(line)
    else stdoutLines.push(line)
  }

  return {
    command,
    language,
    cwd,
    stdoutLines: stdoutLines.slice(0, 120),
    stderrLines: stderrLines.slice(0, 60),
    exitCode,
  }
}

/** Extracts activated skills & memory store/search badges for MemoryChips. */
export function extractMemoryAndSkillChips(
  tools: readonly { name: string; args_preview?: string; content?: string }[],
): { chips: MemoryChip[]; skills: string[]; memories: MemoryChip[] } {
  const seenSkills = new Set<string>()
  const skills: string[] = []
  const memories: MemoryChip[] = []
  const seenMem = new Set<string>()

  for (const t of tools) {
    const lower = t.name.toLowerCase()
    if (SKILL_TOOL_SET.has(lower)) {
      const slug = extractJsonField(t.args_preview, ['name', 'slug', 'skill'])
      if (slug && !seenSkills.has(slug)) {
        seenSkills.add(slug)
        skills.push(slug)
      }
    } else if (MEMORY_TOOL_SET.has(lower)) {
      const title =
        extractJsonField(t.args_preview, ['title', 'query', 'topic', 'key', 'content']) ||
        t.name
      const label = title.length > 42 ? `${title.slice(0, 42)}…` : title
      const change: MemoryChip['change'] =
        lower.includes('store') || lower.includes('save')
          ? 'added'
          : lower.includes('update')
            ? 'updated'
            : 'existing'
      if (label && !seenMem.has(label)) {
        seenMem.add(label)
        memories.push({ id: `mem-${memories.length}`, text: label, change })
      }
    }
  }

  const chips: MemoryChip[] = [
    ...skills.map((s) => ({ id: `skill-${s}`, text: `skill: ${s}`, change: 'added' as const })),
    ...memories,
  ]

  return { chips, skills, memories }
}

/** Converts an ApprovalRecord into PermissionGrant component props. */
export function parsePermissionGrantProps(apr: ApprovalRecord): {
  capability: string
  requester: string
  reach: string[]
} {
  const cmd = extractJsonField(apr.args_preview, ['command', 'cmd', 'path', 'url'])
  const reach: string[] = [
    `Risk level: ${apr.risk}`,
    cmd ? `Target: ${cmd.slice(0, 96)}` : `Arguments: ${(apr.args_preview || '{}').slice(0, 96)}`,
    'Decision synced live with Telegram & WebUI',
  ]
  return {
    capability: `Allow ${apr.tool_name}`,
    requester: 'Zenko runtime',
    reach,
  }
}

/** Splits streaming text into StreamingText segments (marking inline `code` and **bold** spans deterministically). */
export function splitIntoStreamingSegments(text: string): {
  segments: Segment[]
  wordCount: number
} {
  const segments: Segment[] = []
  let idx = 0
  let inCode = false
  let inBold = false
  let buf = ''

  const flush = () => {
    if (!buf) return
    segments.push({ text: buf, mono: inCode, bold: !inCode && inBold })
    buf = ''
  }

  while (idx < text.length) {
    const ch = text[idx]
    if (ch === '`') {
      flush()
      inCode = !inCode
      idx += 1
      continue
    }
    if (!inCode && ch === '*' && text[idx + 1] === '*') {
      flush()
      inBold = !inBold
      idx += 2
      continue
    }
    buf += ch
    idx += 1
  }
  flush()

  const wordCount = segments.reduce(
    (sum, seg) => sum + seg.text.split(' ').filter(Boolean).length,
    0,
  )
  return { segments, wordCount }
}

/** Deterministic calendar day bucket label ("Today", "Yesterday", or "Nov 22, 2026") for DayDivider. */
export function formatDayBucket(iso?: string): string {
  if (!iso) return ''
  const ms = Date.parse(iso)
  if (!Number.isFinite(ms)) return ''
  const date = new Date(ms)
  const now = new Date()
  const dayKey = (d: Date) => `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}`
  if (dayKey(date) === dayKey(now)) return 'Today'
  const yesterday = new Date(now)
  yesterday.setDate(now.getDate() - 1)
  if (dayKey(date) === dayKey(yesterday)) return 'Yesterday'
  try {
    return new Intl.DateTimeFormat(undefined, {
      month: 'short',
      day: 'numeric',
      year: date.getFullYear() === now.getFullYear() ? undefined : 'numeric',
    }).format(date)
  } catch {
    return date.toDateString()
  }
}

/** Recursively extracts plain text from ReactMarkdown children nodes. */
function flattenReactText(node: ReactNode): string {
  if (node == null || typeof node === 'boolean') return ''
  if (typeof node === 'string' || typeof node === 'number') return String(node)
  if (Array.isArray(node)) return node.map(flattenReactText).join('')
  if (isValidElement<{ children?: ReactNode }>(node)) {
    return flattenReactText(node.props.children)
  }
  return ''
}

/** Coerces a clean numeric table cell string to number so DataTable right-aligns & sorts numerically. */
function parseTableCellValue(raw: string): string | number {
  const trimmed = raw.trim()
  if (!trimmed) return ''
  const num = Number(trimmed)
  if (Number.isFinite(num) && trimmed.length <= 16) return num
  return trimmed
}

/**
 * Extracts `{ columns, rows }` from a ReactMarkdown GFM `<table>` element's children
 * so every Markdown table renders using Assistant-UI's sortable, borderless-capable `<DataTable>`.
 */
export function extractMarkdownTableData(
  tableChildren: ReactNode,
): { columns: DataTableColumn[]; rows: DataTableRow[] } | null {
  const headers: string[] = []
  const bodyRows: string[][] = []

  const walk = (nodes: ReactNode, section: 'head' | 'body' | 'none') => {
    Children.forEach(nodes, (child) => {
      if (!isValidElement<{ children?: ReactNode }>(child)) return
      const tag = typeof child.type === 'string' ? child.type : ''
      if (tag === 'thead') {
        walk(child.props.children, 'head')
      } else if (tag === 'tbody') {
        walk(child.props.children, 'body')
      } else if (tag === 'tr') {
        const cells: string[] = []
        Children.forEach(child.props.children, (cell) => {
          if (!isValidElement<{ children?: ReactNode }>(cell)) return
          cells.push(flattenReactText(cell.props.children).trim())
        })
        if (section === 'head' && headers.length === 0) {
          headers.push(...cells)
        } else if (cells.length > 0) {
          bodyRows.push(cells)
        }
      }
    })
  }

  walk(tableChildren, 'none')
  if (headers.length === 0) return null

  const columns: DataTableColumn[] = headers.map((label, idx) => {
    const key = `col_${idx}`
    const allNumeric =
      bodyRows.length > 0 &&
      bodyRows.every((r) => {
        const v = (r[idx] ?? '').trim()
        return v === '' || Number.isFinite(Number(v))
      })
    return {
      key,
      label: label || `Col ${idx + 1}`,
      sortable: true,
      priority: idx === 0 ? 'primary' : 'secondary',
      format: allNumeric ? { kind: 'number' } : { kind: 'text' },
    }
  })

  const rows: DataTableRow[] = bodyRows.map((cells) => {
    const rec: Record<string, string | number> = {}
    columns.forEach((col, idx) => {
      rec[col.key] =
        col.format?.kind === 'number'
          ? parseTableCellValue(cells[idx] ?? '')
          : (cells[idx] ?? '')
    })
    return rec
  })

  return { columns, rows }
}
