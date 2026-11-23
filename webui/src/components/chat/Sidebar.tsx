import { memo } from 'react'
import { motion, AnimatePresence } from 'motion/react'
import { EASE_DRAWER } from '@/lib/ease'
import type { ActiveView, ComponentSlots, Session, UiCustomization } from '@/lib/types'
import { cn } from '@/lib/utils'
import { ZenkoFoxLogo } from '@/components/beui/ZenkoFoxLogo'
import { ZenkoIcon } from '@/components/beui/ZenkoIcon'
import { CompactScrollArea } from '@/components/ui/CompactScrollArea'
import { ZenkoTooltip } from '@/components/ui/Tooltip'

const FRAME_WIDTH: Record<ComponentSlots['left_menubar']['frame_style'], number> = {
  flush_border: 258,
  floating_card: 258,
  compact_rail: 224,
}

const FRAME_CLASS: Record<ComponentSlots['left_menubar']['frame_style'], string> = {
  flush_border: 'h-full border-r border-[var(--border)] bg-[var(--surface)]',
  floating_card:
    'm-2.5 h-[calc(100%-20px)] rounded-2xl border border-[var(--border)] bg-[var(--surface)] shadow-[0_12px_36px_rgba(0,0,0,0.32)]',
  compact_rail: 'h-full border-r border-[var(--border)] bg-[var(--surface)]',
}

const NEW_SESSION_CLASS: Record<ComponentSlots['left_menubar']['new_session_style'], string> = {
  vermilion_soft: 'z-btn z-btn-vermilion-soft w-full justify-start px-3.5 py-2.5 text-[13.5px]',
  solid_primary: 'z-btn z-btn-primary w-full justify-start px-3.5 py-2.5 text-[13.5px]',
  outline_minimal:
    'z-btn z-btn-secondary w-full justify-start bg-transparent px-3.5 py-2.5 text-[13.5px]',
  pill_soft:
    'z-btn z-btn-vermilion-soft w-full justify-start rounded-full! px-4 py-2.5 text-[13.5px]',
}

const SESSION_ACTIVE_CLASS: Record<ComponentSlots['left_menubar']['session_item_style'], string> = {
  soft_fill: 'bg-[var(--card)] text-[var(--fg)] font-medium',
  left_accent_bar: 'border-l-2 border-[var(--accent)] rounded-l-none bg-[var(--card)]/70 text-[var(--fg)] font-medium',
  bordered_card: 'border border-[var(--border)] bg-[var(--card)] text-[var(--fg)] font-medium',
  minimal_text: 'text-[var(--fg)] font-semibold',
}

export const Sidebar = memo(function Sidebar({
  open,
  onToggle,
  sessions,
  activeSessionId,
  activeView,
  ui,
  onSelectSession,
  onNewSession,
  onClearActiveSession,
  onDeleteSession,
  onOpenSettings,
  inlinePreview = false,
}: {
  open: boolean
  onToggle: () => void
  sessions: Session[]
  activeSessionId: string | null
  activeView: ActiveView
  ui: UiCustomization
  onSelectSession: (id: string) => void
  onNewSession: () => void
  onClearActiveSession: () => void
  onDeleteSession: (id: string) => void
  onOpenSettings: () => void
  inlinePreview?: boolean
}) {
  const slot = ui.components.left_menubar
  const frameWidth = FRAME_WIDTH[slot.frame_style] || 258
  const frameClass = FRAME_CLASS[slot.frame_style] || FRAME_CLASS.flush_border
  const newSessionClass = NEW_SESSION_CLASS[slot.new_session_style] || NEW_SESSION_CLASS.vermilion_soft
  const activeItemClass = SESSION_ACTIVE_CLASS[slot.session_item_style] || SESSION_ACTIVE_CLASS.soft_fill

  // Keep Chat Sidebar clean: show root sessions only, with a compact child-count badge when a session spawned subagents.
  const childCounts: Record<string, number> = {}
  for (const s of sessions) {
    if (s.parent_session_id) {
      childCounts[s.parent_session_id] = (childCounts[s.parent_session_id] || 0) + 1
    }
  }
  const rootSessions = sessions.filter((s) => !s.parent_session_id)

  const innerContent = (
    <>
      <div className="flex items-center justify-between px-4 pb-3 pt-4">
        <div className="flex items-center gap-2">
          <span
            className="text-[16px] font-bold tracking-[-0.025em] text-[var(--fg)]"
            style={{ fontFamily: 'var(--font-display)' }}
          >
            Zenko <span style={{ color: 'var(--accent)' }}>Agent</span>
          </span>
          <ZenkoFoxLogo size={20} variant="fox" />
        </div>
        {!inlinePreview && (
          <ZenkoTooltip label="Collapse sidebar" side="right">
            <button
              type="button"
              onClick={onToggle}
              aria-label="Collapse sidebar"
              className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-lg border border-[var(--border)] bg-[var(--card)]/70 text-[var(--muted)] transition-colors hover:text-[var(--fg)]"
            >
              <ZenkoIcon slot="panelLeftClose" icons={ui.icons} size={14} />
            </button>
          </ZenkoTooltip>
        )}
      </div>

      <div className="px-3 pb-3">
        <button
          type="button"
          onClick={onNewSession}
          className={newSessionClass}
          style={slot.new_session_style === 'pill_soft' ? undefined : { borderRadius: 'var(--radius)' }}
        >
          <ZenkoIcon slot="plus" icons={ui.icons} size={15} />
          <span>New Session</span>
        </button>
      </div>

      <CompactScrollArea className="flex-1" viewportClassName="px-2.5 py-1 pr-3.5">
        <div className="flex flex-col gap-0.5">
          {rootSessions.map((s) => {
            const isActive = activeView === 'chat' && s.id === activeSessionId
            const subCount = Math.max(s.subagent_count ?? 0, childCounts[s.id] || 0)
            return (
              <div
                key={s.id}
                onClick={() => onSelectSession(s.id)}
                role="button"
                tabIndex={0}
                onKeyDown={(e) => e.key === 'Enter' && onSelectSession(s.id)}
                className={cn(
                  'group flex cursor-pointer items-center gap-2.5 rounded-[var(--radius-sm)] px-2.5 py-2 text-[13px] transition-colors',
                  isActive
                    ? activeItemClass
                    : 'text-[var(--muted)] hover:bg-[var(--card)]/60 hover:text-[var(--fg)]',
                )}
              >
                <ZenkoIcon
                  slot="chat"
                  icons={ui.icons}
                  size={14}
                  className={cn(
                    'shrink-0 transition-colors',
                    isActive ? 'text-[var(--accent)]' : 'text-[var(--muted)]/70',
                  )}
                />
                <span className="min-w-0 flex-1 truncate">{s.title}</span>
                {subCount > 0 && (
                  <span
                    title={`${subCount} delegated subagent(s)`}
                    className="shrink-0 rounded border border-[var(--border)] bg-[var(--bg)] px-1 py-px font-mono text-[9.5px] text-[var(--muted)]"
                  >
                    {subCount} sub
                  </span>
                )}
                {s.channel === 'telegram' && (
                  <span className="shrink-0 rounded px-1 py-0.5 font-mono text-[10px] text-[var(--muted)]">
                    TG
                  </span>
                )}
                {!inlinePreview && (
                  <ZenkoTooltip label="Delete session" side="right">
                    <button
                      type="button"
                      aria-label="Delete session"
                      onClick={(e) => {
                        e.stopPropagation()
                        onDeleteSession(s.id)
                      }}
                      className="hidden h-5 w-5 shrink-0 cursor-pointer items-center justify-center rounded text-[var(--muted)] hover:text-[var(--fg)] group-hover:flex"
                    >
                      <ZenkoIcon slot="trash" icons={ui.icons} size={12} />
                    </button>
                  </ZenkoTooltip>
                )}
              </div>
            )
          })}
        </div>
      </CompactScrollArea>

      <div className="mt-auto border-t border-[var(--border)] px-3 py-3">
        <button
          type="button"
          onClick={onOpenSettings}
          className={cn(
            'mb-2.5 flex w-full cursor-pointer items-center gap-2.5 rounded-[var(--radius-sm)] px-2 py-1.5 text-[13px] transition-colors',
            activeView === 'setup'
              ? 'text-[var(--fg)]'
              : 'text-[var(--muted)] hover:text-[var(--fg)]',
          )}
        >
          <ZenkoIcon slot="settings" icons={ui.icons} size={15} />
          <span>Settings</span>
        </button>

        <button
          type="button"
          onClick={onClearActiveSession}
          className="flex w-full cursor-pointer items-center gap-2 rounded-[var(--radius-sm)] border border-[var(--border)] bg-[var(--card)]/70 px-3 py-2 text-[12.5px] text-[var(--muted)] transition-colors hover:border-[var(--destructive)]/40 hover:text-[var(--fg)]"
        >
          <ZenkoIcon slot="trash" icons={ui.icons} size={13.5} />
          <span>Clear Active Session</span>
        </button>
      </div>
    </>
  )

  if (inlinePreview) {
    return (
      <div
        className={cn(
          'flex h-[390px] flex-col overflow-hidden',
          slot.frame_style === 'floating_card'
            ? 'rounded-2xl border border-[var(--border)] bg-[var(--surface)] shadow-xl'
            : 'rounded-xl border border-[var(--border)] bg-[var(--surface)]',
        )}
        style={{ width: frameWidth }}
      >
        {innerContent}
      </div>
    )
  }

  return (
    <>
      {!open && (
        <div className="fixed left-4 top-4 z-30">
          <ZenkoTooltip label="Open sidebar" side="right">
            <button
              type="button"
              onClick={onToggle}
              aria-label="Open sidebar"
              className="flex h-9 w-9 cursor-pointer items-center justify-center rounded-xl border border-[var(--border)] bg-[var(--surface)]/90 text-[var(--muted)] backdrop-blur-md transition-colors hover:text-[var(--fg)]"
            >
              <ZenkoIcon slot="panelLeftOpen" icons={ui.icons} size={16} />
            </button>
          </ZenkoTooltip>
        </div>
      )}

      <AnimatePresence initial={false}>
        {open && (
          <motion.aside
            initial={{ width: 0, opacity: 0 }}
            animate={{ width: frameWidth, opacity: 1 }}
            exit={{ width: 0, opacity: 0 }}
            transition={{ duration: 0.22, ease: EASE_DRAWER }}
            className={cn('flex shrink-0 flex-col overflow-hidden', frameClass)}
          >
            {innerContent}
          </motion.aside>
        )}
      </AnimatePresence>
    </>
  )
})
