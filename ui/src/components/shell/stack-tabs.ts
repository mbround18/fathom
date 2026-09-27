import {
  ArchiveIcon,
  CertificateIcon,
  ListChecksIcon,
  PlugsIcon,
  GaugeIcon,
  PulseIcon,
  RocketLaunchIcon,
  ShareNetworkIcon,
  StackIcon,
  TerminalIcon,
  type Icon,
} from '@phosphor-icons/react'

import type { ToolKind } from '@/protocol/types'
import { integrationVisible, VELERO_SECTION } from '@/resources/nav'
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

export const CERTIFICATES_TAB: StackTab = {
  label: 'Certificates',
  icon: CertificateIcon,
  kind: 'tool',
  tool: 'certManager',
}

export const KEDA_TAB: StackTab = {
  label: 'Autoscalers',
  icon: GaugeIcon,
  kind: 'tool',
  tool: 'keda',
}

export const LINTER_TAB: StackTab = {
  label: 'Linter',
  icon: ListChecksIcon,
  kind: 'tool',
  tool: 'linter',
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

export const VELERO_TAB: StackTab = {
  label: 'Velero',
  icon: ArchiveIcon,
  kind: 'tool',
  tool: 'velero',
}

/** Every tool pane reachable from an overlay/"More" surface, in display order. */
export const ALL_TOOL_TABS: readonly StackTab[] = [
  LOGS_TAB,
  EVENTS_TAB,
  FORWARDS_TAB,
  TOPOLOGY_TAB,
  VELERO_TAB,
  KNATIVE_TAB,
  KEDA_TAB,
  LINTER_TAB,
  CERTIFICATES_TAB,
]

/** `tabs` minus the ones hidden for this cluster: Velero until it is detected (043 FR-036). */
export function visibleToolTabs<T extends StackTab>(
  tabs: readonly T[],
  veleroInstalled: boolean | undefined,
): T[] {
  return tabs.filter(
    (t) => t.tool !== 'velero' || integrationVisible(VELERO_SECTION, veleroInstalled),
  )
}

/** `visibleToolTabs` over the store's last Velero scan. */
export function useVisibleToolTabs<T extends StackTab>(tabs: readonly T[]): T[] {
  const veleroInstalled = useStore((s) => s.velero.overview?.installed)
  return visibleToolTabs(tabs, veleroInstalled)
}
