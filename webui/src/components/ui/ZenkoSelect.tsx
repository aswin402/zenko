import { type KeyboardEvent, type ReactNode, useEffect, useMemo, useRef, useState } from 'react'
import { Check, ChevronDown } from 'lucide-react'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/motion/popover'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { cn } from '@/lib/utils'

export interface ZenkoSelectOption {
  value: string
  label: string
  hint?: string
  icon?: ReactNode
}

export interface ZenkoSelectProps {
  value: string
  onChange: (value: string) => void
  options: readonly ZenkoSelectOption[]
  placeholder?: string
  ariaLabel?: string
  title?: string
  disabled?: boolean
  mono?: boolean
  side?: 'top' | 'bottom'
  align?: 'start' | 'center' | 'end'
  className?: string
  contentClassName?: string
  maxHeightClass?: string
}

/**
 * Theme-inheriting dropdown selector built on BeUI Popover + CompactScrollArea.
 * Strictly honors Zenko's active 60/30/10 palette (--bg, --surface, --card, --border, --fg, --muted, --accent)
 * with zero native browser purple/grey select popups.
 */
export function ZenkoSelect({
  value,
  onChange,
  options,
  placeholder = 'Select...',
  ariaLabel,
  title,
  disabled = false,
  mono = false,
  side = 'bottom',
  align = 'start',
  className,
  contentClassName,
  maxHeightClass = 'max-h-64',
}: ZenkoSelectProps) {
  const [open, setOpen] = useState(false)
  const triggerRef = useRef<HTMLButtonElement>(null)
  const [menuMinWidth, setMenuMinWidth] = useState<number>(180)
  const [resolvedSide, setResolvedSide] = useState<'top' | 'bottom'>(side)
  const [highlightedIdx, setHighlightedIdx] = useState<number>(-1)

  const activeIndex = useMemo(
    () => options.findIndex((o) => o.value === value),
    [options, value],
  )
  const activeOption = activeIndex >= 0 ? options[activeIndex] : undefined

  const syncGeometry = () => {
    const rect = triggerRef.current?.getBoundingClientRect()
    if (!rect) return
    if (rect.width > 0) setMenuMinWidth(Math.max(176, Math.round(rect.width)))
    const spaceBelow = window.innerHeight - rect.bottom
    const spaceAbove = rect.top
    const nextSide: 'top' | 'bottom' =
      side === 'top'
        ? spaceAbove < 240 && spaceBelow > spaceAbove
          ? 'bottom'
          : 'top'
        : spaceBelow < 270 && spaceAbove > spaceBelow
          ? 'top'
          : 'bottom'
    setResolvedSide(nextSide)
  }

  useEffect(() => {
    if (!open) return
    syncGeometry()
    setHighlightedIdx(activeIndex >= 0 ? activeIndex : 0)
  }, [open, activeIndex])

  const selectByIndex = (idx: number) => {
    const opt = options[idx]
    if (!opt) return
    onChange(opt.value)
    setOpen(false)
    triggerRef.current?.focus()
  }

  const handleKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (disabled || options.length === 0) return
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      if (!open) {
        setOpen(true)
        return
      }
      setHighlightedIdx((prev) => (prev + 1) % options.length)
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      if (!open) {
        setOpen(true)
        return
      }
      setHighlightedIdx((prev) => (prev - 1 + options.length) % options.length)
    } else if (e.key === 'Home' && open) {
      e.preventDefault()
      setHighlightedIdx(0)
    } else if (e.key === 'End' && open) {
      e.preventDefault()
      setHighlightedIdx(options.length - 1)
    } else if (e.key === 'Enter' && open && highlightedIdx >= 0) {
      e.preventDefault()
      selectByIndex(highlightedIdx)
    } else if (e.key === 'Escape' && open) {
      e.preventDefault()
      setOpen(false)
    }
  }

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        if (disabled) return
        if (next) syncGeometry()
        setOpen(next)
      }}
      side={resolvedSide}
      align={align}
      sideOffset={6}
      panelRadius={12}
      gooStrength={0}
      className="w-full"
    >
      <PopoverTrigger>
        <button
          ref={triggerRef}
          type="button"
          role="combobox"
          aria-expanded={open}
          aria-label={ariaLabel}
          title={title}
          disabled={disabled}
          onKeyDown={handleKeyDown}
          className={cn(
            'flex w-full cursor-pointer items-center justify-between gap-2 rounded-[var(--radius-sm)] border bg-[var(--bg)] px-2.5 py-2 text-left text-[12.5px] text-[var(--fg)] transition-colors outline-none disabled:cursor-not-allowed disabled:opacity-50',
            open
              ? 'border-[var(--accent)]/65 bg-[var(--surface)]'
              : 'border-[var(--border)] hover:border-[var(--muted)]/55',
            mono && 'font-mono text-[11.5px]',
            className,
          )}
        >
          <span className="flex min-w-0 flex-1 items-center gap-2 truncate">
            {activeOption?.icon && (
              <span className="flex shrink-0 items-center text-[var(--accent)]">
                {activeOption.icon}
              </span>
            )}
            <span className={cn('truncate', !activeOption && 'text-[var(--muted)]')}>
              {activeOption?.label ?? placeholder}
            </span>
          </span>
          <ChevronDown
            size={13.5}
            className={cn(
              'shrink-0 text-[var(--muted)] transition-transform duration-200',
              open && 'rotate-180 text-[var(--accent)]',
            )}
          />
        </button>
      </PopoverTrigger>

      <PopoverContent
        className={cn(
          'max-w-[min(92vw,24rem)] rounded-xl border border-[var(--border)] bg-[var(--surface)] p-1 shadow-2xl',
          contentClassName,
        )}
      >
        <div style={{ minWidth: menuMinWidth }} role="listbox" aria-label={ariaLabel}>
          <CompactScrollArea autoHeight viewportClassName={cn(maxHeightClass, 'p-0.5 pr-1.5')}>
            <div className="flex flex-col gap-0.5">
              {options.map((opt, idx) => {
                const isSelected = opt.value === value
                const isHighlighted = idx === highlightedIdx
                return (
                  <button
                    key={opt.value}
                    type="button"
                    role="option"
                    aria-selected={isSelected}
                    onMouseEnter={() => setHighlightedIdx(idx)}
                    onClick={() => selectByIndex(idx)}
                    className={cn(
                      'flex w-full cursor-pointer items-center justify-between gap-2.5 rounded-lg px-2.5 py-1.5 text-left text-[12px] transition-colors outline-none',
                      isSelected
                        ? 'bg-[color-mix(in_srgb,var(--accent)_14%,var(--card))] font-medium text-[var(--fg)]'
                        : isHighlighted
                          ? 'bg-[var(--card)] text-[var(--fg)]'
                          : 'text-[var(--fg)]/85 hover:bg-[var(--card)] hover:text-[var(--fg)]',
                      mono && 'font-mono text-[11.5px]',
                    )}
                  >
                    <span className="flex min-w-0 flex-1 items-center gap-2">
                      {opt.icon && (
                        <span
                          className={cn(
                            'flex shrink-0 items-center',
                            isSelected ? 'text-[var(--accent)]' : 'text-[var(--muted)]',
                          )}
                        >
                          {opt.icon}
                        </span>
                      )}
                      <span className="flex min-w-0 flex-col">
                        <span className="truncate">{opt.label}</span>
                        {opt.hint && (
                          <span className="truncate text-[10.5px] font-normal text-[var(--muted)]">
                            {opt.hint}
                          </span>
                        )}
                      </span>
                    </span>
                    {isSelected && (
                      <Check size={13} strokeWidth={2.4} className="shrink-0 text-[var(--accent)]" />
                    )}
                  </button>
                )
              })}
            </div>
          </CompactScrollArea>
        </div>
      </PopoverContent>
    </Popover>
  )
}
