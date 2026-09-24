import { Fragment, useCallback } from 'react'
import type { Layout } from 'react-resizable-panels'

import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/components/ui/resizable'
import { ScrollFade } from '@/components/ui/scroll-fade'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import type { LayoutProfile } from '@/protocol/types'
import { useStore } from '@/store'

import { TOOL_LABEL } from './labels'
import { PaneHost } from './PaneHost'

/** Only persist real drags; ignore sub-percent jitter from mount and rounding. */
const changed = (a: number, b: number) => Math.abs(a - b) >= 0.5

/**
 * Laptop and up (and the tablet split): one resizable row of slots, plus the logs/events dock
 * underneath from desktop up. Sizes persist to the class's profile on drag end.
 */
export function SlotGrid({ slotCount }: { slotCount?: number }) {
  const widthClass = useStore((s) => s.layout.widthClass)
  const profile = useStore((s) => s.layout.profile)
  const slotPanes = useStore((s) => s.layout.slotPanes)
  const setProfile = useStore((s) => s.setProfile)
  const slots = profile.slots.slice(0, slotCount ?? profile.slots.length)

  const onRowLayout = useCallback(
    (layout: Layout) => {
      const next = profile.slots.map((slot, i) => ({
        ...slot,
        sizePct: layout[`slot-${i}`] ?? slot.sizePct,
      }))
      if (next.some((s, i) => changed(s.sizePct, profile.slots[i]?.sizePct ?? 0))) {
        setProfile({ ...profile, slots: next })
      }
    },
    [profile, setProfile],
  )

  const row = (
    <ResizablePanelGroup
      key={`${widthClass}-${slots.length}`}
      orientation="horizontal"
      onLayoutChanged={onRowLayout}
    >
      {slots.map((slot, i) => {
        const paneId = slotPanes[i]
        return (
          <Fragment key={i}>
            {i > 0 && <ResizableHandle />}
            <ResizablePanel id={`slot-${i}`} defaultSize={`${slot.sizePct}%`} minSize="8%">
              {paneId && <PaneHost paneId={paneId} slotIndex={i} />}
            </ResizablePanel>
          </Fragment>
        )
      })}
    </ResizablePanelGroup>
  )

  if (!profile.dock) return <div className="min-h-0 flex-1">{row}</div>
  return <DockedRow profile={profile}>{row}</DockedRow>
}

function DockedRow({ profile, children }: { profile: LayoutProfile; children: React.ReactNode }) {
  const setProfile = useStore((s) => s.setProfile)
  const dock = profile.dock
  const onLayout = useCallback(
    (layout: Layout) => {
      const h = layout.dock
      if (dock && h !== undefined && changed(h, dock.heightPct)) {
        setProfile({ ...profile, dock: { ...dock, heightPct: h } })
      }
    },
    [dock, profile, setProfile],
  )
  if (!dock) return null

  return (
    <div className="min-h-0 flex-1">
      <ResizablePanelGroup orientation="vertical" onLayoutChanged={onLayout}>
        <ResizablePanel id="row" defaultSize={`${100 - dock.heightPct}%`} minSize="30%">
          {children}
        </ResizablePanel>
        <ResizableHandle />
        <ResizablePanel id="dock" defaultSize={`${dock.heightPct}%`} minSize="10%">
          <Dock />
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  )
}

/** Bottom dock: logs/events tool panes as tabs (desktop+). */
function Dock() {
  const tabs = useStore((s) => s.layout.profile.dock?.tabs ?? [])
  const dockPanes = useStore((s) => s.layout.dockPanes)
  const first = dockPanes[0]
  if (!first) return null

  return (
    <Tabs
      defaultValue={first}
      className="flex h-full min-h-0 flex-col gap-0"
      data-testid="dock-tabs"
    >
      <ScrollFade className="shrink-0 border-b border-border bg-card">
        <TabsList variant="line" className="h-8 w-max min-w-full justify-start px-1">
          {tabs.map((tool, i) => {
            const id = dockPanes[i]
            return id ? (
              <TabsTrigger key={id} value={id} className="flex-none px-3">
                {TOOL_LABEL[tool]}
              </TabsTrigger>
            ) : null
          })}
        </TabsList>
      </ScrollFade>
      {dockPanes.map((id) => (
        <TabsContent key={id} value={id} className="min-h-0 flex-1 overflow-hidden">
          <PaneHost paneId={id} />
        </TabsContent>
      ))}
    </Tabs>
  )
}
