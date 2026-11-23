'use client'

/**
 * KokonutUI — AI Diff Review (adapted strictly to Zenko 60/30/10 theme tokens).
 */

import { Check, ChevronRight, Undo2, X } from 'lucide-react'
import { AnimatePresence, MotionConfig, motion, useReducedMotion } from 'motion/react'
import { type ReactNode, useEffect, useId, useRef, useState } from 'react'
import { cn } from '@/lib/utils'

export interface KokonutDiffLine {
  type: 'add' | 'remove' | 'context'
  text: string
}

export interface KokonutDiffHunk {
  start: number
  lines: KokonutDiffLine[]
}

export type KokonutFileDecision = 'pending' | 'accepted' | 'rejected'

export interface KokonutFileChange {
  path: string
  hunks: KokonutDiffHunk[]
  created?: boolean
}

export interface KokonutAIDiffReviewProps {
  files?: KokonutFileChange[]
  onDecision?: (path: string, decision: KokonutFileDecision) => void
  className?: string
}

const EASE_OUT = [0.23, 1, 0.32, 1] as const
const HEIGHT_SPRING = { type: 'spring', duration: 0.32, bounce: 0 } as const
const ROW_HEIGHT = 40
const SWAP_IN = { duration: 0.18, ease: EASE_OUT } as const
const SWAP_OUT = { duration: 0.12, ease: EASE_OUT } as const

/** Deterministically parse a unified diff string into KokonutFileChange[] (zero regex). */
export function unifiedDiffToKokonutFiles(
  diff: string,
  fallbackPath: string,
): KokonutFileChange[] {
  const lines = diff.split('\n')
  const files: KokonutFileChange[] = []
  let currentPath = fallbackPath
  let currentHunk: KokonutDiffHunk | null = null
  let currentFile: KokonutFileChange | null = null

  const ensureFile = (path: string) => {
    if (!currentFile || currentFile.path !== path) {
      currentFile = { path, hunks: [] }
      files.push(currentFile)
    }
    return currentFile
  }

  for (const raw of lines) {
    if (raw.startsWith('+++ ')) {
      let p = raw.slice(4).trim()
      if (p.startsWith('b/')) p = p.slice(2)
      if (p && p !== '/dev/null') currentPath = p
      currentFile = ensureFile(currentPath)
      currentHunk = null
      continue
    }
    if (raw.startsWith('--- ') || raw.startsWith('diff --git ') || raw.startsWith('index ')) {
      continue
    }
    if (raw.startsWith('@@ ')) {
      const plusIdx = raw.indexOf('+')
      let startNum = 1
      if (plusIdx >= 0) {
        const rest = raw.slice(plusIdx + 1)
        let digits = ''
        for (let i = 0; i < rest.length; i += 1) {
          const ch = rest.charCodeAt(i)
          if (ch >= 48 && ch <= 57) digits += rest[i]
          else break
        }
        if (digits) startNum = Number.parseInt(digits, 10) || 1
      }
      const f = ensureFile(currentPath)
      currentHunk = { start: startNum, lines: [] }
      f.hunks.push(currentHunk)
      continue
    }
    if (!currentHunk) {
      const f = ensureFile(currentPath)
      currentHunk = { start: 1, lines: [] }
      f.hunks.push(currentHunk)
    }
    if (raw.startsWith('+')) {
      currentHunk.lines.push({ type: 'add', text: raw.slice(1) })
    } else if (raw.startsWith('-')) {
      currentHunk.lines.push({ type: 'remove', text: raw.slice(1) })
    } else if (raw.startsWith(' ')) {
      currentHunk.lines.push({ type: 'context', text: raw.slice(1) })
    } else if (raw.length > 0) {
      currentHunk.lines.push({ type: 'context', text: raw })
    }
  }

  return files.length > 0
    ? files
    : [{ path: fallbackPath, hunks: [{ start: 1, lines: [{ type: 'context', text: diff }] }] }]
}

function countLines(hunks: KokonutDiffHunk[]) {
  let added = 0
  let removed = 0
  for (const hunk of hunks) {
    for (const line of hunk.lines) {
      if (line.type === 'add') added += 1
      else if (line.type === 'remove') removed += 1
    }
  }
  return { added, removed }
}

function ChangeBar({ added, removed }: { added: number; removed: number }) {
  const total = added + removed
  const green = total === 0 ? 0 : Math.round((added / total) * 5)
  return (
    <span aria-hidden="true" className="hidden gap-px sm:flex">
      {Array.from({ length: 5 }, (_, i) => (
        <span
          key={i}
          className="size-1.5 rounded-[1px] transition-colors duration-200"
          style={{
            backgroundColor:
              total === 0
                ? 'var(--border)'
                : i < green
                  ? 'var(--success)'
                  : 'var(--destructive)',
          }}
        />
      ))}
    </span>
  )
}

const LINE_STYLE_MAP: Record<KokonutDiffLine['type'], { bg?: string; fg: string; sign: string }> = {
  add: {
    bg: 'color-mix(in srgb, var(--success) 12%, transparent)',
    fg: 'var(--fg)',
    sign: '+',
  },
  remove: {
    bg: 'color-mix(in srgb, var(--destructive) 12%, transparent)',
    fg: 'var(--fg)',
    sign: '−',
  },
  context: {
    fg: 'var(--muted)',
    sign: ' ',
  },
}

function Hunk({ hunk }: { hunk: KokonutDiffHunk }) {
  let number = hunk.start
  return (
    <div className="py-1">
      {hunk.lines.map((line, index) => {
        const lineNumber = line.type === 'remove' ? null : number
        if (line.type !== 'remove') number += 1
        const spec = LINE_STYLE_MAP[line.type]
        return (
          <div
            key={index}
            className="flex font-mono text-[11.5px] leading-5"
            style={{ backgroundColor: spec.bg, color: spec.fg }}
          >
            <span className="w-9 shrink-0 select-none pr-2 text-right tabular-nums text-[var(--muted)]/60">
              {lineNumber}
            </span>
            <span
              className="w-4 shrink-0 select-none text-center font-semibold"
              style={{
                color:
                  line.type === 'add'
                    ? 'var(--success)'
                    : line.type === 'remove'
                      ? 'var(--destructive)'
                      : 'var(--muted)',
              }}
            >
              {spec.sign}
            </span>
            <span className="whitespace-pre pr-3">{line.text || ' '}</span>
          </div>
        )
      })}
    </div>
  )
}

function Collapse({
  open,
  id,
  reduce,
  children,
}: {
  open: boolean
  id: string
  reduce: boolean
  children: ReactNode
}) {
  const innerRef = useRef<HTMLDivElement>(null)
  const [height, setHeight] = useState(0)

  useEffect(() => {
    const element = innerRef.current
    if (!element) return
    const sync = () => setHeight(element.offsetHeight)
    sync()
    const observer = new ResizeObserver(sync)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  return (
    <motion.div
      animate={{ height: open ? height : 0, opacity: open ? 1 : 0 }}
      className="overflow-hidden"
      id={id}
      initial={false}
      transition={
        reduce
          ? { duration: 0 }
          : { height: HEIGHT_SPRING, opacity: open ? SWAP_IN : SWAP_OUT }
      }
    >
      <div ref={innerRef}>{children}</div>
    </motion.div>
  )
}

function FileRow({
  file,
  decision,
  open,
  onToggle,
  onDecide,
}: {
  file: KokonutFileChange
  decision: KokonutFileDecision
  open: boolean
  onToggle: () => void
  onDecide: (decision: KokonutFileDecision) => void
}) {
  const reduce = useReducedMotion() ?? false
  const diffId = useId()
  const { added, removed } = countLines(file.hunks)
  const slash = file.path.lastIndexOf('/')
  const dir = file.path.slice(0, slash + 1)
  const name = file.path.slice(slash + 1)
  const isOpen = open && decision === 'pending'

  return (
    <li className="overflow-hidden border-t border-[var(--border)]/65 first:border-t-0">
      <div
        className="group/row relative flex items-center gap-2 pr-1.5 pl-1"
        style={{ height: ROW_HEIGHT }}
      >
        <button
          aria-controls={diffId}
          aria-expanded={isOpen}
          className="flex h-7 min-w-0 flex-1 cursor-pointer items-center gap-2 rounded-md px-1.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent)]/40 disabled:cursor-default"
          disabled={decision !== 'pending'}
          onClick={onToggle}
          type="button"
        >
          <ChevronRight
            aria-hidden="true"
            className={cn(
              'size-3.5 shrink-0 text-[var(--muted)] transition-[rotate,opacity] duration-200',
              isOpen && 'rotate-90',
              decision !== 'pending' && 'opacity-0',
            )}
          />
          <span
            className={cn(
              'min-w-0 truncate font-mono text-[12.5px] transition-colors duration-200',
              decision === 'rejected' && 'text-[var(--muted)] line-through',
            )}
          >
            <span className="text-[var(--muted)]">{dir}</span>
            <span className={cn(decision !== 'rejected' && 'text-[var(--fg)]')}>{name}</span>
          </span>
          {file.created && (
            <span
              className={cn(
                'shrink-0 rounded-full px-1.5 text-[10px] leading-4 transition-opacity duration-200',
                decision === 'rejected' && 'opacity-0',
              )}
              style={{
                color: 'var(--success)',
                backgroundColor: 'color-mix(in srgb, var(--success) 14%, transparent)',
              }}
            >
              new
            </span>
          )}
        </button>

        <AnimatePresence initial={false} mode="popLayout">
          {decision === 'pending' ? (
            <motion.div
              animate={{ opacity: 1, filter: 'blur(0px)', transition: SWAP_IN }}
              className="flex items-center gap-2"
              exit={{ opacity: 0, filter: 'blur(2px)', transition: SWAP_OUT }}
              initial={{ opacity: 0, filter: 'blur(2px)' }}
              key="counts"
            >
              <span className="font-mono text-[11px] tabular-nums">
                <span style={{ color: 'var(--success)' }}>+{added}</span>{' '}
                <span style={{ color: 'var(--destructive)' }}>−{removed}</span>
              </span>
              <ChangeBar added={added} removed={removed} />
              <span className="flex items-center gap-0.5">
                <button
                  aria-label={`Reject ${file.path}`}
                  className="flex size-7 cursor-pointer items-center justify-center rounded-full text-[var(--muted)] outline-none transition-colors hover:bg-[var(--destructive)]/12 hover:text-[var(--destructive)] focus-visible:ring-2 focus-visible:ring-[var(--accent)]/40"
                  onClick={() => onDecide('rejected')}
                  type="button"
                >
                  <X aria-hidden="true" className="size-3.5" />
                </button>
                <button
                  aria-label={`Accept ${file.path}`}
                  className="flex size-7 cursor-pointer items-center justify-center rounded-full text-[var(--muted)] outline-none transition-colors hover:bg-[var(--success)]/12 hover:text-[var(--success)] focus-visible:ring-2 focus-visible:ring-[var(--accent)]/40"
                  onClick={() => onDecide('accepted')}
                  type="button"
                >
                  <Check aria-hidden="true" className="size-3.5" />
                </button>
              </span>
            </motion.div>
          ) : (
            <motion.div
              animate={{ opacity: 1, filter: 'blur(0px)', transition: SWAP_IN }}
              className="flex items-center gap-1.5"
              exit={{ opacity: 0, filter: 'blur(2px)', transition: SWAP_OUT }}
              initial={{ opacity: 0, filter: 'blur(2px)' }}
              key="decided"
            >
              <span
                className="text-xs"
                style={{ color: decision === 'accepted' ? 'var(--success)' : 'var(--muted)' }}
              >
                {decision === 'accepted' ? 'Accepted' : 'Reverted'}
              </span>
              <button
                aria-label={`Undo for ${file.path}`}
                className="flex size-7 cursor-pointer items-center justify-center rounded-full text-[var(--muted)] outline-none transition-colors hover:bg-[var(--surface)] hover:text-[var(--fg)] focus-visible:ring-2 focus-visible:ring-[var(--accent)]/40"
                onClick={() => onDecide('pending')}
                type="button"
              >
                <Undo2 aria-hidden="true" className="size-3.5" />
              </button>
            </motion.div>
          )}
        </AnimatePresence>
      </div>

      <Collapse id={diffId} open={isOpen} reduce={reduce}>
        <div className="px-2 pb-2">
          <div className="zenko-thin-scroll overflow-x-auto rounded-[var(--radius-sm)] border border-[var(--border)]/70 bg-[var(--bg)]">
            {file.hunks.map((hunk) => (
              <Hunk hunk={hunk} key={hunk.start} />
            ))}
          </div>
        </div>
      </Collapse>
    </li>
  )
}

export default function KokonutAIDiffReview({
  files = [],
  onDecision,
  className,
}: KokonutAIDiffReviewProps) {
  const [decisions, setDecisions] = useState<Record<string, KokonutFileDecision>>({})
  const [opened, setOpened] = useState<Record<string, boolean>>(() =>
    files.length > 0 ? { [files[0].path]: true } : {},
  )

  const decide = (path: string, decision: KokonutFileDecision) => {
    setDecisions((current) => ({ ...current, [path]: decision }))
    onDecision?.(path, decision)
  }
  const decisionOf = (path: string) => decisions[path] ?? 'pending'

  const totals = files.reduce(
    (sum, file) => {
      const { added, removed } = countLines(file.hunks)
      return { added: sum.added + added, removed: sum.removed + removed }
    },
    { added: 0, removed: 0 },
  )
  const pending = files.filter((file) => decisionOf(file.path) === 'pending')
  const accepted = files.filter((file) => decisionOf(file.path) === 'accepted').length
  const allDecided = files.length > 0 && pending.length === 0

  const title = allDecided
    ? `${accepted} of ${files.length} kept`
    : `${files.length} ${files.length === 1 ? 'file' : 'files'} changed`

  return (
    <MotionConfig reducedMotion="user">
      <div className={cn('w-full', className)}>
        <div className="zenko-aui-paper rounded-[var(--radius-lg)] border border-[var(--border)] bg-[var(--surface)]/80 p-1.5">
          <div className="flex h-9 items-center gap-2 px-2.5">
            <output className="text-[13px] font-medium text-[var(--fg)]">{title}</output>
            <span className="font-mono text-[11px] tabular-nums">
              <span style={{ color: 'var(--success)' }}>+{totals.added}</span>{' '}
              <span style={{ color: 'var(--destructive)' }}>−{totals.removed}</span>
            </span>
            {!allDecided && files.length > 0 && (
              <div className="ml-auto flex items-center gap-1">
                <button
                  className="h-7 cursor-pointer rounded-full px-3 text-xs text-[var(--muted)] outline-none transition-colors hover:bg-[var(--card)] hover:text-[var(--fg)]"
                  onClick={() => {
                    for (const f of pending) decide(f.path, 'rejected')
                  }}
                  type="button"
                >
                  Reject all
                </button>
                <button
                  className="h-7 cursor-pointer rounded-full bg-[var(--primary)] px-3 text-xs font-medium text-[var(--primary-fg)] outline-none transition-opacity hover:opacity-90"
                  onClick={() => {
                    for (const f of pending) decide(f.path, 'accepted')
                  }}
                  type="button"
                >
                  Accept all
                </button>
              </div>
            )}
          </div>
          <ul className="rounded-[var(--radius)] border border-[var(--border)]/75 bg-[var(--card)]">
            {files.map((file) => (
              <FileRow
                key={file.path}
                decision={decisionOf(file.path)}
                file={file}
                onDecide={(d) => decide(file.path, d)}
                onToggle={() =>
                  setOpened((cur) => ({ ...cur, [file.path]: !(cur[file.path] ?? false) }))
                }
                open={Boolean(opened[file.path])}
              />
            ))}
          </ul>
        </div>
      </div>
    </MotionConfig>
  )
}

export { KokonutAIDiffReview }
