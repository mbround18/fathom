import { MagnifyingGlassIcon } from '@phosphor-icons/react'
import { useCallback, useLayoutEffect, useRef, type UIEvent } from 'react'

import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'
import { useStore, type PaneId, type PaneState } from '@/store'

import { paneTitle, TOOL_LABEL } from './labels'
import { NavList } from './NavList'
import { Disconnected, Empty, Loading, MetricsUnavailable } from './PaneStates'

interface PaneHostProps {
  paneId: PaneId
  /** Position among visible slots; omitted for dock tabs. */
  slotIndex?: number
  className?: string
}

/**
 * Renders one pane by kind. All view state (search, selection, scroll) lives in
 * `store.panes[paneId]`, so the host can unmount on re-layout and come back intact (FR-007).
 */
export function PaneHost({ paneId, slotIndex, className }: PaneHostProps) {
  const pane = useStore((s) => s.panes[paneId])
  const updatePane = useStore((s) => s.updatePane)
  const scrollRef = useRef<HTMLDivElement>(null)
  const frame = useRef(0)

  // Restore scroll before paint; read once so scroll writes do not feed back.
  useLayoutEffect(() => {
    const el = scrollRef.current
    if (el) el.scrollTop = useStore.getState().panes[paneId]?.scrollTop ?? 0
    return () => cancelAnimationFrame(frame.current)
  }, [paneId])

  const onScroll = useCallback(
    (e: UIEvent<HTMLDivElement>) => {
      const top = e.currentTarget.scrollTop
      cancelAnimationFrame(frame.current)
      frame.current = requestAnimationFrame(() => updatePane(paneId, { scrollTop: top }))
    },
    [paneId, updatePane],
  )

  if (!pane) return null
  const title = paneTitle(pane)

  return (
    <section
      aria-label={title}
      data-slot="pane"
      data-slot-index={slotIndex}
      data-pane-kind={pane.kind}
      data-pane-id={pane.id}
      className={cn('flex h-full min-h-0 min-w-0 flex-col bg-background', className)}
    >
      <header className="flex h-8 shrink-0 items-center gap-2 border-b border-border bg-card px-2">
        {slotIndex !== undefined && (
          <span aria-hidden="true" className="text-[0.65rem] text-accent-neon">
            {String(slotIndex + 1).padStart(2, '0')}
          </span>
        )}
        <h2 className="min-w-0 truncate font-display text-xs font-semibold tracking-wider uppercase">
          {title}
        </h2>
      </header>
      {pane.kind === 'list' && <PaneSearch pane={pane} />}
      <div ref={scrollRef} onScroll={onScroll} className="min-h-0 flex-1 overflow-auto">
        <PaneBody pane={pane} />
      </div>
    </section>
  )
}

function PaneSearch({ pane }: { pane: PaneState }) {
  const updatePane = useStore((s) => s.updatePane)
  return (
    <div className="relative shrink-0 border-b border-border">
      <MagnifyingGlassIcon
        aria-hidden="true"
        className="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground"
      />
      <Input
        type="search"
        aria-label={`Filter ${paneTitle(pane)}`}
        placeholder="filter…"
        value={pane.search}
        onChange={(e) => updatePane(pane.id, { search: e.target.value })}
        className="h-[var(--touch-target)] rounded-none border-0 pl-7 focus-visible:ring-inset"
      />
    </div>
  )
}

function PaneBody({ pane }: { pane: PaneState }) {
  const disconnected = useStore((s) => s.conn.status === 'disconnected')

  switch (pane.kind) {
    case 'nav':
      return <NavList />
    case 'list':
      if (disconnected) return <Disconnected />
      return pane.route ? (
        <Loading label={`Loading ${pane.route.section}`} />
      ) : (
        <Empty title="Pick a section" detail="Choose a resource type from the navigator." />
      )
    case 'inspector':
      return <Empty title="Nothing selected" detail="Select a resource to inspect it." />
    case 'tool':
      if (pane.tool === 'metrics') return <MetricsUnavailable />
      return (
        <Empty
          title={pane.tool ? TOOL_LABEL[pane.tool] : 'Tool'}
          detail="No data yet. Connect to a cluster to populate this view."
        />
      )
  }
}
