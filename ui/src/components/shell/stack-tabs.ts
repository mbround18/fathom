import {
  PlugsIcon,
  PulseIcon,
  RocketLaunchIcon,
  ShareNetworkIcon,
  StackIcon,
  TerminalIcon,
  type Icon,
} from '@phosphor-icons/react'

import type { ToolKind } from '@/protocol/types'
import { useStore, type PaneKind } from '@/store'

export interface StackTab {
  label: string
  icon: Icon
  kind: PaneKind
  tool?: ToolKind
}

export const RESOURCES_TAB: StackTab = { label: 'Resources', icon: StackIcon, kind: 'list' }
export const LOGS_TAB: StackTab = { label: 'Logs', icon: TerminalIcon, kind: 'tool', tool: 'logs' }
export const EVENTS_TAB: StackTab = {
  label: 'Events',
  icon: PulseIcon,
  kind: 'tool',
  tool: 'events',
}
export const FORWARDS_TAB: StackTab = {
  label: 'Forwards',
  icon: PlugsIcon,
  kind: 'tool',
  tool: 'portforwards',
}
export const TOPOLOGY_TAB: StackTab = {
  label: 'Topology',
  icon: ShareNetworkIcon,
  kind: 'tool',
  tool: 'topology',
}
export const KNATIVE_TAB: StackTab = {
  label: 'Knative',
  icon: RocketLaunchIcon,
  kind: 'tool',
  tool: 'knative',
}

/** Which tab the stack root belongs to: tool panes by tool, everything else is Resources. */
export function useActiveTab(tabs: readonly StackTab[]): StackTab | undefined {
  return useStore((s) => {
    const root = s.layout.stack[0]
    const pane = root ? s.panes[root] : undefined
    if (pane?.kind === 'tool') return tabs.find((t) => t.tool === pane.tool)
    return tabs.find((t) => t.kind === 'list')
  })
}
