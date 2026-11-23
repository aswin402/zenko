export interface FloatingPos {
  x: number
  y: number
}

export interface FloatingSize {
  width: number
  height: number
}

export const TERMINAL_MONITOR_CONFIG = {
  width: 396,
  height: 468,
  collapsedHeight: 46,
  margin: 16,
  dockClearance: 76,
  maxBufferChars: 36_000,
  recentItemsLimit: 18,
  storageKey: 'zenko.activity_monitor.pos.v1',
} as const

/**
 * Deterministic 4-state DFA stripping ANSI/VT100 escape sequences (CSI, OSC, 2-byte ESC)
 * in a single O(n) scan with zero regex.
 */
export function stripAnsiStateMachine(input: string): string {
  const enum AnsiState {
    Normal = 0,
    Esc = 1,
    Csi = 2,
    Osc = 3,
    OscEsc = 4,
  }

  let state = AnsiState.Normal
  let out = ''

  for (let i = 0; i < input.length; i += 1) {
    const code = input.charCodeAt(i)
    switch (state) {
      case AnsiState.Normal:
        if (code === 0x1b) {
          state = AnsiState.Esc
        } else if (code === 0x0d) {
          // Normalize CRLF -> LF; collapse bare CR overwrites cleanly
          if (i + 1 < input.length && input.charCodeAt(i + 1) === 0x0a) {
            // skip CR before LF
          } else {
            out += '\n'
          }
        } else {
          out += input[i]
        }
        break
      case AnsiState.Esc:
        if (code === 0x5b) {
          // '[' -> CSI sequence
          state = AnsiState.Csi
        } else if (code === 0x5d) {
          // ']' -> OSC sequence
          state = AnsiState.Osc
        } else {
          state = AnsiState.Normal
        }
        break
      case AnsiState.Csi:
        if (code >= 0x40 && code <= 0x7e) {
          state = AnsiState.Normal
        }
        break
      case AnsiState.Osc:
        if (code === 0x07) {
          state = AnsiState.Normal
        } else if (code === 0x1b) {
          state = AnsiState.OscEsc
        }
        break
      case AnsiState.OscEsc:
        state = code === 0x5c ? AnsiState.Normal : AnsiState.Osc
        break
    }
  }
  return out
}

/**
 * Bounded terminal ring-buffer append: strips ANSI codes and caps memory cleanly at
 * a newline boundary so long build logs never bloat browser RAM.
 */
export function appendBoundedTerminal(
  existing: string,
  rawChunk: string,
  maxChars: number = TERMINAL_MONITOR_CONFIG.maxBufferChars,
): string {
  const clean = stripAnsiStateMachine(rawChunk)
  if (!clean) return existing
  const combined = existing + clean
  if (combined.length <= maxChars) return combined
  const cut = combined.length - maxChars
  const nextNl = combined.indexOf('\n', cut)
  return nextNl !== -1 && nextNl - cut < 240 ? combined.slice(nextNl + 1) : combined.slice(cut)
}

/**
 * Pure arithmetic viewport clamping — guarantees draggable windows stay reachable
 * across any drag or window resize.
 */
export function clampFloatingPosition(
  pos: FloatingPos,
  size: FloatingSize,
  viewport: FloatingSize,
  margin: number = TERMINAL_MONITOR_CONFIG.margin,
): FloatingPos {
  const maxX = Math.max(margin, viewport.width - size.width - margin)
  const maxY = Math.max(margin, viewport.height - size.height - margin)
  return {
    x: Math.round(Math.min(maxX, Math.max(margin, pos.x))),
    y: Math.round(Math.min(maxY, Math.max(margin, pos.y))),
  }
}

export function defaultFloatingPosition(
  size: FloatingSize,
  viewport: FloatingSize,
): FloatingPos {
  return clampFloatingPosition(
    {
      x: viewport.width - size.width - TERMINAL_MONITOR_CONFIG.dockClearance,
      y: viewport.height - size.height - 28,
    },
    size,
    viewport,
  )
}

export function loadSavedFloatingPosition(
  size: FloatingSize,
  viewport: FloatingSize,
): FloatingPos {
  try {
    const raw = window.localStorage.getItem(TERMINAL_MONITOR_CONFIG.storageKey)
    if (raw) {
      const parsed = JSON.parse(raw)
      if (typeof parsed?.x === 'number' && typeof parsed?.y === 'number') {
        return clampFloatingPosition({ x: parsed.x, y: parsed.y }, size, viewport)
      }
    }
  } catch {
    /* ignore storage errors */
  }
  return defaultFloatingPosition(size, viewport)
}

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB'] as const

export function formatBytesCompact(bytes: number): string {
  const safe = Math.max(0, Number.isFinite(bytes) ? bytes : 0)
  let val = safe
  let unitIdx = 0
  while (val >= 1024 && unitIdx < BYTE_UNITS.length - 1) {
    val /= 1024
    unitIdx += 1
  }
  return unitIdx === 0 ? `${Math.round(val)}B` : `${val.toFixed(1)}${BYTE_UNITS[unitIdx]}`
}

export function formatElapsedCompact(
  startedIso?: string | null,
  finishedIso?: string | null,
  nowMs: number = Date.now(),
): string {
  if (!startedIso) return '0s'
  const start = Date.parse(startedIso)
  if (!Number.isFinite(start)) return '0s'
  const end = finishedIso ? Date.parse(finishedIso) : nowMs
  const totalSec = Math.max(0, Math.floor((Math.max(start, end) - start) / 1000))
  if (totalSec < 60) return `${totalSec}s`
  const mins = Math.floor(totalSec / 60)
  const rem = totalSec % 60
  return rem === 0 ? `${mins}m` : `${mins}m ${rem}s`
}
