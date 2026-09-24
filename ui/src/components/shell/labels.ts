import type { ToolKind } from '@/protocol/types'
import { sectionLabel } from '@/resources/nav'
import type { PaneState } from '@/store'

export const TOOL_LABEL: Record<ToolKind, string> = {
  overview: 'Overview',
  metrics: 'Metrics',
  flux: 'Flux',
  hubble: 'Hubble',
  logs: 'Logs',
  events: 'Events',
  portforwards: 'Forwards',
  topology: 'Topology',
  knative: 'Knative',
}

export function paneTitle(pane: PaneState): string {
  switch (pane.kind) {
    case 'nav':
      return 'Navigator'
    case 'list':
      return pane.route ? sectionLabel(pane.route.section) : 'Resources'
    case 'inspector':
      return pane.target?.name ?? 'Inspector'
    case 'tool':
      return pane.tool ? TOOL_LABEL[pane.tool] : 'Tool'
  }
}
