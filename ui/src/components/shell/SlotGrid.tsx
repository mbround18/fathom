import { Fragment, useCallback, useEffect, useState } from 'react'
import { type Layout, type LayoutChangedMeta, useGroupRef } from 'react-resizable-panels'
import { useShallow } from 'zustand/react/shallow'

import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/components/ui/resizable'
import { ScrollFade } from '@/components/ui/scroll-fade'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { dockTabs } from '@/layout/defaults'
import type { LayoutProfile } from '@/protocol/types'
import { useStore } from '@/store'

import { TOOL_LABEL } from './labels'
import { PaneHost } from './PaneHost'

/** Only persist real drags; ignore sub-percent jitter from mount and rounding. */
const changed = (a: number, b: number) => Math.abs(a - b) >= 0.5

/**
 * Laptop and up (and the tablet split): one resizable row of slots, plus the logs/events dock
 * underneath from laptop up. Sizes persist to the class's profile on drag end.
 *
 * The row is never remounted to apply sizes (feedback #2: a width-class change used to remount
 * every slot, resetting inspector tabs and restarting log streams). A class change, a slot
 * count change or a loaded `ui` config instead applies the profile's sizes through the group's
 * imperative `setLayout`; panes keep their identity.
 */
export function SlotGrid({ slotCount }: { slotCount?: number }) {
  const widthClass = useStore((s) => s.layout.widthClass)
  const profile = useStore((s) => s.layout.profile)
  const slotPanes = useStore((s) => s.layout.slotPanes)
  const setProfile = useStore((s) => s.setProfile)
  // Saved sizes arrive with `UiConfigLoaded` after first paint; re-apply them then.
  const uiLoaded = useStore((s) => s.prefs.uiEpoch ?? 0)
  const slots = profile.slots.slice(0, slotCount ?? profile.slots.length)
  const groupRef = useGroupRef()
  // The sizes this class's profile wants, as a stable string so the effect below runs only when
  // they (or the class, the slot count, a loaded config) actually change.
  const targetSizes = slots.map((slot) => slot.sizePct).join(',')

  useEffect(() => {
    const target: Layout = {}
    targetSizes.split(',').forEach((pct, i) => {
      target[`slot-${i}`] = Number(pct)
    })
    let frame = 0
    let tries = 0
    const apply = () => {
      const group = groupRef.current
      if (!group) return
      const current = group.getLayout()
      // 044 (FR-010): a class change with a different slot count runs this effect while the group
      // still holds the previous class's panels (ultra has five, wide four), and a layout that
      // names fewer panels than the group has throws and takes the whole row down. Wait for the
      // counts to agree instead — bounded, so a group that never settles just keeps its sizes.
      if (Object.keys(current).length !== Object.keys(target).length) {
        if (tries++ < 30) frame = requestAnimationFrame(apply)
        return
      }
      if (Object.keys(target).some((id) => changed(current[id] ?? -100, target[id] ?? 0))) {
        group.setLayout(target)
      }
    }
    apply()
    return () => {
      if (frame) cancelAnimationFrame(frame)
    }
  }, [groupRef, targetSizes, widthClass, uiLoaded])

  const onRowLayout = useCallback(
    (layout: Layout, meta: LayoutChangedMeta) => {
      // Only a drag or resize key is the operator's choice. Mount, constraint recompute and
      // our own `setLayout` must not write one class's sizes into another class's profile.
      if (!meta.isUserInteraction) return
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
    <ResizablePanelGroup groupRef={groupRef} orientation="horizontal" onLayoutChanged={onRowLayout}>
      {slots.map((slot, i) => {
        const paneId = slotPanes[i]
        return (
          <Fragment key={i}>
            {i > 0 && <ResizableHandle aria-label={`Resize section ${i} and section ${i + 1}`} />}
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
  const groupRef = useGroupRef()
  const heightPct = dock?.heightPct
  // Same as the row: a class change applies the new dock height in place.
  useEffect(() => {
    const group = groupRef.current
    if (!group || heightPct === undefined) return
    if (changed(group.getLayout().dock ?? -100, heightPct)) {
      group.setLayout({ row: 100 - heightPct, dock: heightPct })
    }
  }, [groupRef, heightPct])
  const onLayout = useCallback(
    (layout: Layout, meta: LayoutChangedMeta) => {
      if (!meta.isUserInteraction) return
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
      <ResizablePanelGroup groupRef={groupRef} orientation="vertical" onLayoutChanged={onLayout}>
        <ResizablePanel id="row" defaultSize={`${100 - dock.heightPct}%`} minSize="30%">
          {children}
        </ResizablePanel>
        <ResizableHandle aria-label="Resize tool dock height" />
        <ResizablePanel id="dock" defaultSize={`${dock.heightPct}%`} minSize="10%">
          <Dock />
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  )
}

/** The daemon is connected for the active context, so Cluster history belongs in the dock. */
function useDaemonAvailable(): boolean {
  return useStore((s) => {
    const ctx = s.session.activeContext
    return ctx !== undefined && s.daemon[ctx]?.status?.state === 'available'
  })
}

/**
 * Bottom dock (laptop+): only the streams that follow what the operator is doing (043 FR-024):
 * Logs, Events, and Cluster history while the daemon is available. Tools a saved layout still
 * lists are dropped by `dockTabs`; their homes are navigator sections now.
 */
function Dock() {
  const dock = useStore((s) => s.layout.profile.dock)
  const dockPanes = useStore((s) => s.layout.dockPanes)
  const paneTools = useStore(useShallow((s) => s.layout.dockPanes.map((id) => s.panes[id]?.tool)))
  const daemonAvailable = useDaemonAvailable()
  const [selected, setSelected] = useState<string | undefined>()
  const tabs = dock ? dockTabs(dock, daemonAvailable) : []
  const shown = tabs.flatMap((tool) => {
    const id = dockPanes[paneTools.indexOf(tool)]
    return id ? [{ tool, id }] : []
  })
  const first = shown[0]
  if (!first) return null
  // A tab that went away (the daemon disconnected) falls back to the first one.
  const value = shown.some((t) => t.id === selected) ? selected : first.id

  return (
    <Tabs
      value={value}
      onValueChange={setSelected}
      className="flex h-full min-h-0 flex-col gap-0"
      data-testid="dock-tabs"
    >
      <ScrollFade className="shrink-0 border-b border-border bg-card">
        <TabsList variant="line" className="h-[max(2rem,26px)] w-max min-w-full justify-start px-1">
          {shown.map(({ tool, id }) => (
            <TabsTrigger key={id} value={id} data-tool={tool} className="flex-none px-3">
              {TOOL_LABEL[tool]}
            </TabsTrigger>
          ))}
        </TabsList>
      </ScrollFade>
      {shown.map(({ id }) => (
        <TabsContent key={id} value={id} className="min-h-0 flex-1 overflow-hidden">
          <PaneHost paneId={id} />
        </TabsContent>
      ))}
    </Tabs>
  )
}
