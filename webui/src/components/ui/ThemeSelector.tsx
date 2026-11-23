import { useId } from 'react'
import { Check, Moon, Sun } from 'lucide-react'
import { LayoutGroup, motion, useReducedMotion } from 'motion/react'
import { SPRING_LAYOUT, SPRING_PRESS } from '@/lib/ease'
import type { ThemePresetMeta } from '@/lib/types'
import { cn } from '@/lib/utils'

/**
 * RareUI-inspired ThemeSelector (`swamimalode07/rare-ui/theme-selector`).
 * Displays unified color theme families where every theme supports both Dark & Light modes,
 * with live 4:3 workspace previews, spring selection ring, and keyboard navigation.
 */
export function ThemeSelector({
  themes,
  value,
  mode,
  onChange,
  labels = true,
  className,
}: {
  themes: ThemePresetMeta[]
  value: string | null
  mode: 'dark' | 'light'
  onChange: (id: string, targetMode?: 'dark' | 'light') => void
  labels?: boolean
  className?: string
}) {
  const groupId = useId()
  const reduce = useReducedMotion()

  const handleKeyDown = (idx: number, e: React.KeyboardEvent<HTMLDivElement>) => {
    if (themes.length === 0) return
    const keyMap: Record<string, number> = {
      ArrowRight: (idx + 1) % themes.length,
      ArrowDown: (idx + 1) % themes.length,
      ArrowLeft: (idx - 1 + themes.length) % themes.length,
      ArrowUp: (idx - 1 + themes.length) % themes.length,
      Home: 0,
      End: themes.length - 1,
    }
    const nextIdx = keyMap[e.key]
    if (nextIdx !== undefined) {
      e.preventDefault()
      onChange(themes[nextIdx].id, mode)
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      onChange(themes[idx].id, mode)
    }
  }

  return (
    <LayoutGroup id={groupId}>
      <div
        role="radiogroup"
        aria-label="Global color theme"
        className={cn('grid grid-cols-2 gap-4 sm:grid-cols-3 lg:grid-cols-4', className)}
      >
        {themes.map((t, idx) => {
          const selected = value === t.id
          const swatch = mode === 'light' ? t.light : t.dark
          const { bg, surface, border, fg, primary: accent } = swatch

          return (
            <div
              key={t.id}
              role="radio"
              tabIndex={selected || idx === 0 ? 0 : -1}
              aria-checked={selected}
              onKeyDown={(e) => handleKeyDown(idx, e)}
              className="group relative flex flex-col gap-2 text-left focus:outline-none"
            >
              {/* 4:3 Miniature Workspace Preview Card */}
              <motion.button
                type="button"
                tabIndex={-1}
                whileTap={reduce ? undefined : { scale: 0.98 }}
                transition={SPRING_PRESS}
                onClick={() => onChange(t.id, mode)}
                className="relative aspect-[4/3] w-full cursor-pointer overflow-hidden rounded-xl border p-2.5 text-left transition-transform duration-150 group-hover:-translate-y-0.5"
                style={{
                  backgroundColor: bg,
                  borderColor: selected ? accent : border,
                }}
              >
                {selected && (
                  <motion.span
                    layoutId={`rare-theme-ring-${groupId}`}
                    transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
                    className="pointer-events-none absolute inset-0 rounded-xl ring-2 ring-inset"
                    style={{ '--tw-ring-color': accent } as React.CSSProperties}
                  />
                )}

                {/* Mini Workspace Chrome */}
                <div className="flex h-full w-full gap-2">
                  <div
                    className="flex w-[32%] flex-col justify-between rounded-lg border p-1.5"
                    style={{ backgroundColor: surface, borderColor: border }}
                  >
                    <div className="flex flex-col gap-1.5">
                      <div className="flex items-center gap-1">
                        <span className="h-1.5 w-1.5 rounded-full" style={{ backgroundColor: accent }} />
                        <span className="h-1 w-7 rounded-full opacity-75" style={{ backgroundColor: fg }} />
                      </div>
                      <span className="h-2.5 w-full rounded" style={{ backgroundColor: border }} />
                      <span className="h-1 w-4/5 rounded-full opacity-35" style={{ backgroundColor: fg }} />
                      <span className="h-1 w-3/5 rounded-full opacity-25" style={{ backgroundColor: fg }} />
                    </div>
                    <span className="h-1 w-1/2 rounded-full opacity-25" style={{ backgroundColor: fg }} />
                  </div>

                  <div className="flex flex-1 flex-col justify-between py-0.5">
                    <div className="flex flex-col gap-1.5">
                      <div className="flex justify-end">
                        <span
                          className="h-3 w-3/5 rounded-md border"
                          style={{ backgroundColor: surface, borderColor: border }}
                        />
                      </div>
                      <div className="flex items-start gap-1">
                        <span
                          className="mt-0.5 h-2 w-2 shrink-0 rounded-full"
                          style={{ backgroundColor: accent }}
                        />
                        <div className="flex flex-1 flex-col gap-1">
                          <span className="h-1 w-full rounded-full opacity-65" style={{ backgroundColor: fg }} />
                          <span className="h-1 w-4/5 rounded-full opacity-40" style={{ backgroundColor: fg }} />
                        </div>
                      </div>
                    </div>

                    <div
                      className="flex items-center justify-between rounded-lg border px-1.5 py-1"
                      style={{ backgroundColor: surface, borderColor: border }}
                    >
                      <span className="h-1 w-1/2 rounded-full opacity-35" style={{ backgroundColor: fg }} />
                      <span className="h-2.5 w-2.5 rounded-sm" style={{ backgroundColor: accent }} />
                    </div>
                  </div>
                </div>

                {/* Selected checkmark badge */}
                {selected && (
                  <span
                    className="absolute right-2 top-2 flex h-4.5 w-4.5 items-center justify-center rounded-full shadow-xs"
                    style={{ backgroundColor: accent, color: swatch.primary_fg }}
                  >
                    <Check size={10} strokeWidth={3} />
                  </span>
                )}
              </motion.button>

              {/* Theme Name + Inline Dark / Light Switch */}
              {labels && (
                <div className="flex items-center justify-between gap-2 px-0.5">
                  <button
                    type="button"
                    onClick={() => onChange(t.id, mode)}
                    className={cn(
                      'cursor-pointer truncate text-[12.5px] font-medium transition-colors',
                      selected ? 'text-[var(--fg)]' : 'text-[var(--muted)] group-hover:text-[var(--fg)]',
                    )}
                  >
                    {t.label}
                  </button>

                  <div
                    className={cn(
                      'flex shrink-0 items-center rounded-md border border-[var(--border)] bg-[var(--surface)] p-0.5 transition-opacity',
                      selected ? 'opacity-100' : 'opacity-65 group-hover:opacity-100',
                    )}
                  >
                    <button
                      type="button"
                      aria-label={`${t.label} Dark mode`}
                      onClick={(e) => {
                        e.stopPropagation()
                        onChange(t.id, 'dark')
                      }}
                      className={cn(
                        'flex h-5 w-5 cursor-pointer items-center justify-center rounded-[4px] transition-colors',
                        selected && mode === 'dark'
                          ? 'bg-[var(--accent)] text-[var(--primary-fg)]'
                          : 'text-[var(--muted)] hover:text-[var(--fg)]',
                      )}
                    >
                      <Moon size={10.5} />
                    </button>
                    <button
                      type="button"
                      aria-label={`${t.label} Light mode`}
                      onClick={(e) => {
                        e.stopPropagation()
                        onChange(t.id, 'light')
                      }}
                      className={cn(
                        'flex h-5 w-5 cursor-pointer items-center justify-center rounded-[4px] transition-colors',
                        selected && mode === 'light'
                          ? 'bg-[var(--accent)] text-[var(--primary-fg)]'
                          : 'text-[var(--muted)] hover:text-[var(--fg)]',
                      )}
                    >
                      <Sun size={10.5} />
                    </button>
                  </div>
                </div>
              )}
            </div>
          )
        })}
      </div>
    </LayoutGroup>
  )
}
