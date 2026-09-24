import { ArrowLeftIcon } from '@phosphor-icons/react'

import { Button } from '@/components/ui/button'
import { useStore } from '@/store'

import { PaneHost } from './PaneHost'
import { SlotGrid } from './SlotGrid'
import {
  EVENTS_TAB,
  FORWARDS_TAB,
  LOGS_TAB,
  RESOURCES_TAB,
  TOPOLOGY_TAB,
  useActiveTab,
} from './stack-tabs'
import { StackTabs } from './StackTabs'

const TABLET_TABS = [RESOURCES_TAB, LOGS_TAB, EVENTS_TAB, FORWARDS_TAB, TOPOLOGY_TAB] as const

/**
 * Tablet: a tab strip on top. Resources shows the list | inspector split; the tool tabs take the
 * full width. The navigator lives in the drawer.
 *
 * A tool tab (Logs/Events/Forwards/Topology) has no dedicated inspector slot of its own, so
 * `openInspector` (007-topology-graph-view: node click) falls back to pushing the inspector onto
 * `layout.stack`, same as phone — the top of the stack is what must render, and a "Back" control
 * is needed to leave it (mirrors `PhoneStack.tsx`).
 */
export function TabletTabs() {
  const active = useActiveTab(TABLET_TABS)
  const stack = useStore((s) => s.layout.stack)
  const popPane = useStore((s) => s.popPane)
  const closeInspector = useStore((s) => s.closeInspector)
  const top = stack.at(-1)
  // A tool root outside the tab strip (e.g. the daemon timeline) still takes the full width.
  const rootIsTool = useStore((s) => {
    const root = s.layout.stack[0]
    return root ? s.panes[root]?.kind === 'tool' : false
  })
  const topIsInspector = useStore((s) => (top ? s.panes[top]?.kind === 'inspector' : false))

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <StackTabs tabs={TABLET_TABS} className="border-b" />
      {(active?.kind === 'tool' || rootIsTool) && top ? (
        <div className="flex min-h-0 flex-1 flex-col">
          {stack.length > 1 && (
            <Button
              variant="ghost"
              onClick={topIsInspector ? closeInspector : popPane}
              className="h-[var(--touch-target)] shrink-0 justify-start border-b border-border px-3"
            >
              <ArrowLeftIcon data-icon="inline-start" /> Back
            </Button>
          )}
          <div className="min-h-0 flex-1">
            <PaneHost key={top} paneId={top} slotIndex={0} />
          </div>
        </div>
      ) : (
        <SlotGrid />
      )}
    </div>
  )
}
