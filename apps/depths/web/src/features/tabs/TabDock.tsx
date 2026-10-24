/**
 * The open tabs (FR-024): side by side in resizable panes at wide widths, one pane with every tab
 * in its strip at narrow widths. Loaded lazily, once the first tab exists; importing it starts the
 * streams.
 */
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@companion/ui'
import { Fragment } from 'react'

import { orderedTabs, type TabSpec } from './layout'
import { useTabs } from './store'
import './streams'
import { TabHost } from './TabHost'
import { useWide } from './useWide'

export default function TabDock() {
  const layout = useTabs((s) => s.layout)
  const setSizes = useTabs((s) => s.setSizes)
  const wide = useWide()

  if (layout.tabs.length === 0) return null

  if (!wide) {
    return (
      <div data-dock="narrow" className="h-full min-h-0">
        <TabHost paneId="narrow" index={null} paneCount={1} tabs={orderedTabs(layout)} active={layout.focused} />
      </div>
    )
  }

  const byId = new Map(layout.tabs.map((t) => [t.id, t]))
  const panes = layout.panes
  const even = 100 / Math.max(panes.length, 1)
  return (
    <div data-dock="wide" className="h-full min-h-0">
      <ResizablePanelGroup
        // A new set of panes is a new group, sized from what was saved for each.
        key={panes.map((p) => p.id).join(',')}
        orientation="horizontal"
        onLayoutChanged={(l, meta) => {
          if (meta.isUserInteraction) setSizes(l)
        }}
      >
        {panes.map((p, i) => (
          <Fragment key={p.id}>
            {i > 0 && <ResizableHandle withHandle aria-label={`Resize panes ${String(i)} and ${String(i + 1)}`} />}
            <ResizablePanel id={p.id} defaultSize={String(layout.sizes[p.id] ?? even)} minSize="10">
              <TabHost
                paneId={p.id}
                index={i}
                paneCount={panes.length}
                tabs={p.tabs.flatMap((id): TabSpec[] => {
                  const t = byId.get(id)
                  return t ? [t] : []
                })}
                active={p.active}
              />
            </ResizablePanel>
          </Fragment>
        ))}
      </ResizablePanelGroup>
    </div>
  )
}
