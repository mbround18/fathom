import type { ToolKind } from '@/protocol/types'
import { sectionLabel } from '@/resources/nav'
import type { PaneState } from '@/store'

export const TOOL_LABEL: Record<ToolKind, string> = {
  overview: 'Overview',
  metrics: 'Metrics',
  flux: 'Flux',
  hubble: 'Hubble',
  keda: 'Autoscalers',
  logs: 'Logs',
  events: 'Events',
  portforwards: 'Forwards',
  topology: 'Topology',
  daemon: 'Cluster history',
  velero: 'Velero',
  knative: 'Knative',
  certManager: 'Certificates',
  vulnerabilities: 'Vulnerabilities',
  linter: 'Linter',
}

export function paneTitle(pane: PaneState): string {
  switch (pane.kind) {
    case 'nav':
      return 'Sections'
    case 'list':
      return pane.route ? sectionLabel(pane.route.section) : 'Resources'
    case 'inspector':
      return pane.target?.name ?? 'Inspector'
    case 'tool':
      return pane.tool ? TOOL_LABEL[pane.tool] : 'Tool'
  }
}
