'use client'

import { useCallback, useEffect, useState, type KeyboardEvent, type ReactNode } from 'react'
import {
  LayoutGrid,
  MessageSquare,
  PanelLeftClose,
  PanelLeftOpen,
  Settings,
  Trash2,
} from 'lucide-react'
import { cn } from '@/lib/utils'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { ZenkoTooltip } from '@/components/ui/Tooltip'

export interface ResponsiveSidebarItem {
  id: string
  label: string
  badge?: string | number
  icon?: ReactNode
  channel?: string
}

export type ResponsiveSessionItemStyle =
  | 'soft_fill'
  | 'left_accent_bar'
  | 'bordered_card'
  | 'minimal_text'

export interface ResponsiveSidebarProps {
  /** Brand text displayed in the top header. */
  brand?: ReactNode
  /** Brand mark / emblem rendered beside the brand title. */
  logo?: ReactNode
  /** Vertical list of navigation or session items. */
  items?: ResponsiveSidebarItem[]
  /** Active item id. */
  activeId?: string | null
  /** Controlled collapsed state (optional). */
  collapsed?: boolean
  /** Initial collapsed state when uncontrolled. */
  defaultCollapsed?: boolean
  /** Callback when collapsed state toggles. */
  onCollapsedChange?: (collapsed: boolean) => void
  /** Callback when an item is selected. */
  onSelectItem?: (id: string) => void
  /** Optional delete action per item. */
  onDeleteItem?: (id: string) => void
  /** Optional top primary CTA slot (e.g. New Session button) receiving collapsed flag. */
  headerAction?: (collapsed: boolean) => ReactNode
  /** Optional footer slot (e.g. Settings & Clear Session) receiving collapsed flag. */
  footer?: (collapsed: boolean) => ReactNode
  /** Active row highlight style matching Zenko's Left Menubar customization. */
  sessionItemStyle?: ResponsiveSessionItemStyle
  /** Enable smooth scroll inertia on the item list. */
  smoothScroll?: boolean
  /** Disable tooltip portals when rendered inside Studio previews. */
  inlinePreview?: boolean
  /** Optional label for the sidebar navigation landmark. */
  ariaLabel?: string
  className?: string
}

const DEFAULT_ITEMS: ResponsiveSidebarItem[] = [
  { id: 'overview', label: 'Dashboard', icon: <LayoutGrid size={15} /> },
  { id: 'messages', label: 'Conversations', icon: <MessageSquare size={15} />, badge: 3 },
  { id: 'settings', label: 'Settings', icon: <Settings size={15} /> },
]

const ACTIVE_ROW_STYLE: Record<ResponsiveSessionItemStyle, string> = {
  soft_fill: 'bg-[var(--card)]/90 text-[var(--fg)] font-medium',
  left_accent_bar:
    'border-l-2 border-[var(--accent)] rounded-l-none bg-[var(--card)]/80 text-[var(--fg)] font-medium',
  bordered_card: 'border border-[var(--border)] bg-[var(--card)]/90 text-[var(--fg)] font-medium',
  minimal_text: 'text-[var(--fg)] font-semibold',
}

/**
 * Responsive Semi-Transparent Glass Sidebar.
 * Features smooth width collapse/expand transitions, frosted theme-aware translucency,
 * slim Zenko scrollbars, and full ARIA + keyboard navigation (Enter, Space, Tab, Escape).
 */
export function ResponsiveSidebar({
  brand = 'Zenko',
  logo,
  items = DEFAULT_ITEMS,
  activeId = null,
  collapsed: controlledCollapsed,
  defaultCollapsed = false,
  onCollapsedChange,
  onSelectItem,
  onDeleteItem,
  headerAction,
  footer,
  sessionItemStyle = 'soft_fill',
  smoothScroll = true,
  inlinePreview = false,
  ariaLabel = 'Workspace navigation',
  className,
}: ResponsiveSidebarProps) {
  const [internalCollapsed, setInternalCollapsed] = useState(defaultCollapsed)
  const isCollapsed = controlledCollapsed ?? internalCollapsed

  const setCollapsed = useCallback(
    (next: boolean) => {
      if (controlledCollapsed === undefined) setInternalCollapsed(next)
      onCollapsedChange?.(next)
    },
    [controlledCollapsed, onCollapsedChange],
  )

  useEffect(() => {
    if (inlinePreview) return
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key === 'Escape' && !isCollapsed && document.activeElement?.closest('[data-responsive-sidebar]')) {
        setCollapsed(true)
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [inlinePreview, isCollapsed, setCollapsed])

  const handleItemKeyDown = (event: KeyboardEvent<HTMLElement>, id: string) => {
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      onSelectItem?.(id)
    }
  }

  const activeClass = ACTIVE_ROW_STYLE[sessionItemStyle] || ACTIVE_ROW_STYLE.soft_fill

  return (
    <aside
      data-responsive-sidebar=""
      aria-label={ariaLabel}
      className={cn(
        'relative flex h-full shrink-0 flex-col overflow-hidden border-r border-[var(--border)]/80 bg-[var(--surface)]/78 text-[var(--fg)] shadow-[0_12px_36px_rgba(0,0,0,0.22)] backdrop-blur-xl transition-[width] duration-300 ease-[cubic-bezier(0.22,1,0.36,1)] motion-reduce:transition-none',
        isCollapsed ? 'w-[68px]' : 'w-[258px]',
        className,
      )}
    >
      {/* Top Brand Header + Collapse Toggle */}
      <div
        className={cn(
          'flex shrink-0 items-center border-b border-[var(--border)]/60 py-3.5 transition-all duration-200',
          isCollapsed ? 'justify-center px-2' : 'justify-between px-3.5',
        )}
      >
        {!isCollapsed && (
          <div className="flex min-w-0 items-center gap-2 overflow-hidden">
            {logo}
            <span
              className="truncate text-[15px] font-bold tracking-[-0.025em] text-[var(--fg)]"
              style={{ fontFamily: 'var(--font-display)' }}
            >
              {brand}
            </span>
          </div>
        )}
        <ZenkoTooltip
          label={isCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          side="right"
          disabled={inlinePreview}
        >
          <button
            type="button"
            aria-label={isCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
            aria-expanded={!isCollapsed}
            onClick={() => setCollapsed(!isCollapsed)}
            className="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-[var(--radius-sm)] border border-[var(--border)]/80 bg-[var(--card)]/75 text-[var(--muted)] outline-none transition-colors hover:border-[var(--accent)]/45 hover:text-[var(--fg)] focus-visible:ring-2 focus-visible:ring-[var(--accent)]/50"
          >
            {isCollapsed ? <PanelLeftOpen size={14.5} /> : <PanelLeftClose size={14.5} />}
          </button>
        </ZenkoTooltip>
      </div>

      {/* Primary Action CTA (e.g. New Session) */}
      {headerAction && (
        <div className={cn('shrink-0 pt-3 pb-2', isCollapsed ? 'flex justify-center px-2' : 'px-3')}>
          {headerAction(isCollapsed)}
        </div>
      )}

      {/* Navigation / Session Items List */}
      <CompactScrollArea
        smooth={smoothScroll}
        className="min-h-0 flex-1"
        viewportClassName={cn('py-1.5', isCollapsed ? 'px-2' : 'px-2.5 pr-3.5')}
      >
        <nav aria-label={ariaLabel} className="flex flex-col gap-0.5">
          {items.map((item) => {
            const isActive = activeId === item.id
            const initialGlyph = (item.label.trim().charAt(0) || '•').toUpperCase()
            const row = (
              <div
                key={item.id}
                role="button"
                tabIndex={0}
                aria-current={isActive ? 'page' : undefined}
                aria-label={item.label}
                onClick={() => onSelectItem?.(item.id)}
                onKeyDown={(e) => handleItemKeyDown(e, item.id)}
                className={cn(
                  'group flex cursor-pointer items-center rounded-[var(--radius-sm)] text-[13px] outline-none transition-colors focus-visible:ring-2 focus-visible:ring-[var(--accent)]/50',
                  isCollapsed ? 'h-9 justify-center px-0' : 'gap-2 px-2.5 py-2',
                  isActive
                    ? activeClass
                    : 'text-[var(--muted)] hover:bg-[var(--card)]/65 hover:text-[var(--fg)]',
                )}
              >
                {isCollapsed ? (
                  item.icon ? (
                    <span className="flex items-center justify-center">{item.icon}</span>
                  ) : (
                    <span
                      className={cn(
                        'flex h-6 w-6 items-center justify-center rounded-md font-mono text-[11px] font-semibold',
                        isActive
                          ? 'bg-[var(--accent)]/16 text-[var(--accent)]'
                          : 'bg-[var(--card)]/70 text-[var(--muted)] group-hover:text-[var(--fg)]',
                      )}
                    >
                      {initialGlyph}
                    </span>
                  )
                ) : (
                  <>
                    {item.icon && <span className="shrink-0 opacity-80">{item.icon}</span>}
                    <span className="min-w-0 flex-1 truncate">{item.label}</span>
                    {item.badge !== undefined && (
                      <span className="shrink-0 rounded border border-[var(--border)] bg-[var(--bg)]/80 px-1.5 py-px font-mono text-[9.5px] text-[var(--muted)]">
                        {item.badge}
                      </span>
                    )}
                    {item.channel === 'telegram' && (
                      <span className="shrink-0 font-mono text-[10px] text-[var(--muted)]">TG</span>
                    )}
                    {onDeleteItem && !inlinePreview && (
                      <button
                        type="button"
                        aria-label={`Delete ${item.label}`}
                        onClick={(e) => {
                          e.stopPropagation()
                          onDeleteItem(item.id)
                        }}
                        className="hidden h-5 w-5 shrink-0 cursor-pointer items-center justify-center rounded text-[var(--muted)] hover:text-[var(--destructive)] group-hover:flex"
                      >
                        <Trash2 size={12} />
                      </button>
                    )}
                  </>
                )}
              </div>
            )

            return isCollapsed ? (
              <ZenkoTooltip key={item.id} label={item.label} side="right" disabled={inlinePreview}>
                {row}
              </ZenkoTooltip>
            ) : (
              row
            )
          })}
        </nav>
      </CompactScrollArea>

      {/* Footer Actions */}
      {footer && (
        <div
          className={cn(
            'mt-auto shrink-0 border-t border-[var(--border)]/60 py-3',
            isCollapsed ? 'flex flex-col items-center px-2' : 'px-3',
          )}
        >
          {footer(isCollapsed)}
        </div>
      )}
    </aside>
  )
}

export default ResponsiveSidebar
