import { useStore } from '@/store'

import { PaneHost } from './PaneHost'
import { SlotGrid } from './SlotGrid'
import { EVENTS_TAB, FORWARDS_TAB, LOGS_TAB, RESOURCES_TAB, useActiveTab } from './stack-tabs'
import { StackTabs } from './StackTabs'

const TABLET_TABS = [RESOURCES_TAB, LOGS_TAB, EVENTS_TAB, FORWARDS_TAB] as const

/**
 * Tablet: a tab strip on top. Resources shows the list | inspector split; the tool tabs take the
 * full width. The navigator lives in the drawer.
 */
export function TabletTabs() {
  const active = useActiveTab(TABLET_TABS)
  const root = useStore((s) => s.layout.stack[0])

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <StackTabs tabs={TABLET_TABS} className="border-b" />
      {active?.kind === 'tool' && root ? (
        <div className="min-h-0 flex-1">
          <PaneHost key={root} paneId={root} slotIndex={0} />
        </div>
      ) : (
        <SlotGrid />
      )}
    </div>
  )
}
