/**
 * One pane: its tab strip and the active tab. Only the active tab is mounted, so a hidden log tab
 * renders nothing and a hidden stats tab stops watching; a shell keeps its session in the store
 * and replays its output when shown again.
 */
import { lazy, Suspense } from 'react'

import type { TabSpec } from './layout'
import { LogsTab } from './LogsTab'
import { StatsTab } from './StatsTab'
import { TabStrip } from './TabStrip'

const ShellTab = lazy(() => import('./ShellTab'))

export interface TabHostProps {
  paneId: string
  index: number | null
  paneCount: number
  tabs: TabSpec[]
  active: string | null
}

function Body({ tab }: { tab: TabSpec }) {
  switch (tab.kind) {
    case 'logs':
      return <LogsTab tab={tab} />
    case 'stats':
      return <StatsTab tab={tab} />
    case 'shell':
      return (
        <Suspense
          fallback={
            <p role="status" data-pane-state="loading" className="p-2 text-xs text-muted-foreground">
              Loading terminal…
            </p>
          }
        >
          <ShellTab tab={tab} />
        </Suspense>
      )
  }
}

export function TabHost({ paneId, index, paneCount, tabs, active }: TabHostProps) {
  const current = tabs.find((t) => t.id === active) ?? tabs[0]
  return (
    <section data-pane={paneId} aria-label={index === null ? 'Tabs' : `Pane ${String(index + 1)}`} className="flex h-full min-h-0 min-w-0 flex-col">
      <TabStrip paneId={paneId} index={index} paneCount={paneCount} tabs={tabs} active={current?.id ?? null} />
      <div
        role="tabpanel"
        id={`tabpanel-${paneId}`}
        aria-labelledby={current ? `tab-${current.id}` : undefined}
        className="min-h-0 flex-1"
      >
        {current && <Body key={current.id} tab={current} />}
      </div>
    </section>
  )
}
