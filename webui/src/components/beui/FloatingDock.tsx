import { memo } from 'react'
import { motion, useReducedMotion } from 'motion/react'
import { SPRING_LAYOUT, SPRING_PRESS } from '@/lib/ease'
import type { ActiveView, ComponentSlots, UiCustomization } from '@/lib/types'
import { cn } from '@/lib/utils'
import { ThemeToggle } from '@/components/motion/theme-toggle'
import {
  Skiper98VerticalDock,
  VtooltipContent,
  VtooltipItem,
  type Skiper98DockItem,
} from '@/components/skiperui/vertical-tooltip-menu'
import { ZenkoTooltip } from '@/components/ui/Tooltip'
import { GlassDock, type GlassDockItem } from '@/components/vengeanceui/glass-dock'
import { ZenkoIcon } from './ZenkoIcon'

interface DockEntry {
  id: ActiveView
  label: string
  icon: string
}

const DOCK_ITEMS: DockEntry[] = [
  { id: 'telemetry', label: 'Dashboard', icon: 'grid' },
  { id: 'chat', label: 'Chat Workbench', icon: 'chat' },
]

const DOCK_SHELL_CLASS: Record<ComponentSlots['right_menubar']['dock_style'], string> = {
  floating_capsule:
    'fixed right-4 top-4 z-30 rounded-[18px] border border-[var(--border)] bg-[var(--surface)]/96 p-1.5 shadow-[0_12px_36px_rgba(0,0,0,0.42)]',
  attached_rail:
    'fixed right-0 top-0 z-30 h-full justify-start border-l border-[var(--border)] bg-[var(--surface)] px-1.5 py-3',
  glass_pill:
    'fixed right-4 top-4 z-30 rounded-full border border-[var(--border)] bg-[var(--card)]/72 p-1.5 shadow-[0_14px_40px_rgba(0,0,0,0.36)] backdrop-blur-md',
  vengeance_glass_dock: 'fixed right-4 top-4 z-30',
  skiper98_vtooltip: 'fixed right-4 top-4 z-30',
  minimal_strip:
    'fixed right-3 top-4 z-30 rounded-xl border border-transparent bg-transparent p-1',
}

const DOCK_PREVIEW_SHELL_CLASS: Record<ComponentSlots['right_menubar']['dock_style'], string> = {
  floating_capsule:
    'rounded-[18px] border border-[var(--border)] bg-[var(--surface)]/96 p-1.5 shadow-[0_12px_36px_rgba(0,0,0,0.38)]',
  attached_rail:
    'rounded-r-xl border border-[var(--border)] bg-[var(--surface)] px-1.5 py-3',
  glass_pill:
    'rounded-full border border-[var(--border)] bg-[var(--card)]/72 p-1.5 backdrop-blur-md shadow-lg',
  vengeance_glass_dock: '',
  skiper98_vtooltip: '',
  minimal_strip: 'rounded-xl border border-dashed border-[var(--border)]/60 bg-transparent p-1',
}

/**
 * Minimal floating vertical right-edge icon dock powered by spring-gliding `layoutId` highlight
 * and theme-inheriting `<ZenkoTooltip />` hover hints.
 */
export const FloatingDock = memo(function FloatingDock({
  activeView,
  onSelectView,
  ui,
  onToggleMode,
  onOpenQuickCustomizer,
  monitorOpen = false,
  activeBgCount = 0,
  onToggleMonitor,
  collapsed,
  onToggleCollapse,
  inlinePreview = false,
}: {
  activeView: ActiveView
  onSelectView: (v: ActiveView) => void
  ui: UiCustomization
  onToggleMode: () => void
  onOpenQuickCustomizer: () => void
  monitorOpen?: boolean
  activeBgCount?: number
  onToggleMonitor?: () => void
  collapsed: boolean
  onToggleCollapse: () => void
  inlinePreview?: boolean
}) {
  const reduce = useReducedMotion()
  const slot = ui.components.right_menubar
  const layoutId = inlinePreview ? 'zenko-dock-preview-pill' : 'zenko-dock-active-pill'

  if (collapsed && !inlinePreview) {
    return (
      <div className="fixed right-4 top-4 z-30">
        <ZenkoTooltip label="Expand navigation dock" side="left">
          <button
            type="button"
            onClick={onToggleCollapse}
            aria-label="Expand navigation dock"
            className="flex h-10 w-10 cursor-pointer items-center justify-center rounded-2xl border border-[var(--border)] bg-[var(--surface)]/96 text-[var(--muted)] transition-colors hover:text-[var(--fg)]"
          >
            <ZenkoIcon slot="panelRightOpen" icons={ui.icons} size={17} />
          </button>
        </ZenkoTooltip>
      </div>
    )
  }

  const renderActivePill = () => {
    if (slot.active_indicator === 'solid_accent') {
      return (
        <motion.span
          layoutId={layoutId}
          transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
          className="pointer-events-none absolute inset-0 rounded-[11px]"
          style={{ backgroundColor: 'var(--primary)' }}
        />
      )
    }
    if (slot.active_indicator === 'side_bar') {
      return (
        <motion.span
          layoutId={layoutId}
          transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
          className="pointer-events-none absolute right-0 top-1.5 bottom-1.5 w-[2.5px] rounded-full"
          style={{ backgroundColor: 'var(--accent)' }}
        />
      )
    }
    if (slot.active_indicator === 'dot_indicator') {
      return (
        <motion.span
          layoutId={layoutId}
          transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
          className="pointer-events-none absolute bottom-1 h-1 w-1 rounded-full"
          style={{ backgroundColor: 'var(--accent)' }}
        />
      )
    }
    return (
      <motion.span
        layoutId={layoutId}
        transition={reduce ? { duration: 0 } : SPRING_LAYOUT}
        className="pointer-events-none absolute inset-0 rounded-[11px] border"
        style={{
          backgroundColor: 'color-mix(in srgb, var(--accent) 14%, transparent)',
          borderColor: 'color-mix(in srgb, var(--accent) 52%, transparent)',
        }}
      />
    )
  }

  const activeTextClass =
    slot.active_indicator === 'solid_accent' ? 'text-[var(--primary-fg)]' : 'text-[var(--accent)]'

  const shellClass = inlinePreview
    ? DOCK_PREVIEW_SHELL_CLASS[slot.dock_style] || DOCK_PREVIEW_SHELL_CLASS.floating_capsule
    : DOCK_SHELL_CLASS[slot.dock_style] || DOCK_SHELL_CLASS.floating_capsule

  if (slot.dock_style === 'vengeance_glass_dock') {
    const glassItems: GlassDockItem[] = [
      ...DOCK_ITEMS.map((item) => ({
        id: item.id,
        title: item.label,
        icon: <ZenkoIcon slot={item.icon} icons={ui.icons} size={17} />,
        active: activeView === item.id,
        onClick: () => onSelectView(item.id),
      })),
      {
        id: 'zenko_task',
        title: activeBgCount > 0 ? `zenko_task (${activeBgCount} running)` : 'zenko_task',
        icon: <ZenkoIcon slot="terminal" icons={ui.icons} size={17} />,
        active: monitorOpen,
        badge: activeBgCount,
        onClick: onToggleMonitor,
      },
      {
        id: 'appearance',
        title: 'Customization',
        icon: <ZenkoIcon slot="palette" icons={ui.icons} size={17} />,
        active: activeView === 'appearance',
        onClick: () =>
          activeView === 'appearance' ? onOpenQuickCustomizer() : onSelectView('appearance'),
      },
    ]

    return (
      <aside aria-label="Workspace navigation dock" className={shellClass}>
        <GlassDock
          orientation="vertical"
          items={glassItems}
          activeIndicator={slot.active_indicator}
          layoutId={layoutId}
          disableTooltip={inlinePreview}
          trailing={
            <>
              <div className="my-0.5 h-px w-5 bg-[var(--border)]" />
              <ZenkoTooltip
                label={ui.mode === 'light' ? 'Switch to Dark mode' : 'Switch to Light mode'}
                side="left"
                disabled={inlinePreview}
              >
                <ThemeToggle
                  mode={ui.mode === 'light' ? 'light' : 'dark'}
                  variant={ui.motion.theme_transition || 'circle-blur'}
                  start="top-right"
                  onToggle={onToggleMode}
                  aria-label="Toggle Dark / Light mode"
                  className="h-9 w-9 rounded-[13px] text-[var(--muted)] transition-colors hover:bg-[var(--surface)]/80 hover:text-[var(--fg)]"
                  iconClassName="size-[17px]"
                />
              </ZenkoTooltip>
              {!inlinePreview && (
                <ZenkoTooltip label="Collapse navigation dock" side="left">
                  <button
                    type="button"
                    aria-label="Collapse navigation dock"
                    onClick={onToggleCollapse}
                    className="flex h-9 w-9 cursor-pointer items-center justify-center rounded-[13px] text-[var(--muted)] transition-colors hover:bg-[var(--surface)]/80 hover:text-[var(--fg)]"
                  >
                    <ZenkoIcon slot="panelRightClose" icons={ui.icons} size={17} />
                  </button>
                </ZenkoTooltip>
              )}
            </>
          }
        />
      </aside>
    )
  }

  if (slot.dock_style === 'skiper98_vtooltip') {
    const skiperItems: Skiper98DockItem[] = [
      ...DOCK_ITEMS.map((item) => ({
        id: item.id,
        label: item.label,
        icon: <ZenkoIcon slot={item.icon} icons={ui.icons} size={17} />,
        active: activeView === item.id,
        onClick: () => onSelectView(item.id),
      })),
      {
        id: 'zenko_task',
        label: activeBgCount > 0 ? `zenko_task (${activeBgCount} running)` : 'zenko_task',
        icon: <ZenkoIcon slot="terminal" icons={ui.icons} size={17} />,
        active: monitorOpen,
        badge: activeBgCount,
        onClick: onToggleMonitor,
      },
      {
        id: 'appearance',
        label: 'Customization',
        shortcut: '⌘.',
        icon: <ZenkoIcon slot="palette" icons={ui.icons} size={17} />,
        active: activeView === 'appearance',
        onClick: () =>
          activeView === 'appearance' ? onOpenQuickCustomizer() : onSelectView('appearance'),
      },
    ]

    return (
      <aside aria-label="Workspace navigation dock" className={shellClass}>
        <Skiper98VerticalDock
          items={skiperItems}
          activeIndicator={slot.active_indicator}
          layoutId={layoutId}
          disableLabels={false}
          trailing={
            <>
              <div className="my-0.5 h-px w-5 bg-[var(--border)]" />
              <VtooltipItem id="theme-mode">
                <ThemeToggle
                  mode={ui.mode === 'light' ? 'light' : 'dark'}
                  variant={ui.motion.theme_transition || 'circle-blur'}
                  start="top-right"
                  onToggle={onToggleMode}
                  aria-label="Toggle Dark / Light mode"
                  className="h-9 w-9 rounded-[12px] text-[var(--muted)] transition-colors hover:bg-[var(--card)]/85 hover:text-[var(--fg)]"
                  iconClassName="size-[17px]"
                />
                <VtooltipContent
                  id="theme-mode"
                  label={ui.mode === 'light' ? 'Switch to Dark mode' : 'Switch to Light mode'}
                  side="left"
                />
              </VtooltipItem>
              {!inlinePreview && (
                <VtooltipItem id="collapse-dock">
                  <button
                    type="button"
                    aria-label="Collapse navigation dock"
                    onClick={onToggleCollapse}
                    className="flex h-9 w-9 cursor-pointer items-center justify-center rounded-[12px] text-[var(--muted)] transition-colors hover:bg-[var(--card)]/85 hover:text-[var(--fg)]"
                  >
                    <ZenkoIcon slot="panelRightClose" icons={ui.icons} size={17} />
                  </button>
                  <VtooltipContent id="collapse-dock" label="Collapse dock" side="left" />
                </VtooltipItem>
              )}
            </>
          }
        />
      </aside>
    )
  }

  return (
    <aside
      aria-label="Workspace navigation dock"
      className={cn('flex flex-col items-center gap-1', shellClass)}
    >
      {DOCK_ITEMS.map((item) => {
        const isActive = activeView === item.id
        return (
          <ZenkoTooltip key={item.id} label={item.label} side="left" disabled={inlinePreview}>
            <motion.button
              type="button"
              aria-label={item.label}
              whileTap={reduce ? undefined : { scale: 0.93 }}
              transition={SPRING_PRESS}
              onClick={() => onSelectView(item.id)}
              className={cn(
                'relative flex h-9 w-9 cursor-pointer items-center justify-center rounded-[11px] transition-colors',
                isActive
                  ? activeTextClass
                  : 'text-[var(--muted)] hover:bg-[var(--card)]/80 hover:text-[var(--fg)]',
              )}
            >
              {isActive && renderActivePill()}
              <span className="relative z-10">
                <ZenkoIcon slot={item.icon} icons={ui.icons} size={17} />
              </span>
            </motion.button>
          </ZenkoTooltip>
        )
      })}

      <div className="my-1 h-px w-5 bg-[var(--border)]" />

      <ZenkoTooltip
        label={activeBgCount > 0 ? `zenko_task (${activeBgCount} running)` : 'zenko_task'}
        side="left"
        disabled={inlinePreview}
      >
        <motion.button
          type="button"
          aria-label="zenko_task background runner"
          aria-pressed={monitorOpen}
          whileTap={reduce ? undefined : { scale: 0.93 }}
          transition={SPRING_PRESS}
          onClick={onToggleMonitor}
          className={cn(
            'relative flex h-9 w-9 cursor-pointer items-center justify-center rounded-[11px] transition-colors',
            monitorOpen
              ? 'text-[var(--accent)]'
              : 'text-[var(--muted)] hover:bg-[var(--card)]/80 hover:text-[var(--fg)]',
          )}
          style={
            monitorOpen
              ? {
                  backgroundColor: 'color-mix(in srgb, var(--accent) 14%, transparent)',
                  boxShadow: 'inset 0 0 0 1px color-mix(in srgb, var(--accent) 48%, transparent)',
                }
              : undefined
          }
        >
          <span className="relative z-10">
            <ZenkoIcon slot="terminal" icons={ui.icons} size={17} />
          </span>
          {activeBgCount > 0 && (
            <span
              className="absolute right-1 top-1 z-20 flex h-3.5 min-w-[14px] items-center justify-center rounded-full px-1 font-mono text-[9px] font-semibold leading-none"
              style={{
                backgroundColor: 'var(--accent)',
                color: 'var(--primary-fg)',
              }}
            >
              {activeBgCount > 9 ? '9+' : activeBgCount}
            </span>
          )}
        </motion.button>
      </ZenkoTooltip>

      <ZenkoTooltip label="Customization" shortcut="⌘." side="left" disabled={inlinePreview}>
        <button
          type="button"
          aria-label="Customization"
          onClick={() =>
            activeView === 'appearance' ? onOpenQuickCustomizer() : onSelectView('appearance')
          }
          className={cn(
            'relative flex h-9 w-9 cursor-pointer items-center justify-center rounded-[11px] transition-colors',
            activeView === 'appearance'
              ? activeTextClass
              : 'text-[var(--muted)] hover:bg-[var(--card)]/80 hover:text-[var(--fg)]',
          )}
        >
          {activeView === 'appearance' && renderActivePill()}
          <span className="relative z-10">
            <ZenkoIcon slot="palette" icons={ui.icons} size={17} />
          </span>
        </button>
      </ZenkoTooltip>

      <ZenkoTooltip
        label={ui.mode === 'light' ? 'Switch to Dark mode' : 'Switch to Light mode'}
        side="left"
        disabled={inlinePreview}
      >
        <ThemeToggle
          mode={ui.mode === 'light' ? 'light' : 'dark'}
          variant={ui.motion.theme_transition || 'circle-blur'}
          start="top-right"
          onToggle={onToggleMode}
          aria-label="Toggle Dark / Light mode"
          className="h-9 w-9 rounded-[11px] text-[var(--muted)] transition-colors hover:bg-[var(--card)]/80 hover:text-[var(--fg)]"
          iconClassName="size-[17px]"
        />
      </ZenkoTooltip>

      {!inlinePreview && (
        <ZenkoTooltip label="Collapse navigation dock" side="left">
          <button
            type="button"
            aria-label="Collapse navigation dock"
            onClick={onToggleCollapse}
            className="flex h-9 w-9 cursor-pointer items-center justify-center rounded-[11px] text-[var(--muted)] transition-colors hover:bg-[var(--card)]/80 hover:text-[var(--fg)]"
          >
            <ZenkoIcon slot="panelRightClose" icons={ui.icons} size={17} />
          </button>
        </ZenkoTooltip>
      )}
    </aside>
  )
})
