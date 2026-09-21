import type { SectionKey, ToolKind } from '@/protocol/types'
import type { PaneState } from '@/store'

export const TOOL_LABEL: Record<ToolKind, string> = {
  overview: 'Overview',
  metrics: 'Metrics',
  flux: 'Flux',
  logs: 'Logs',
  events: 'Events',
  portforwards: 'Forwards',
}

// TODO(protocol): replace with generated SectionKey values from src/protocol/v1/route.rs
export const NAV_SECTIONS: readonly { group: string; sections: readonly SectionKey[] }[] = [
  { group: 'Cluster', sections: ['Overview', 'Nodes', 'Namespaces', 'Events'] },
  {
    group: 'Workloads',
    sections: ['Pods', 'Deployments', 'StatefulSets', 'DaemonSets', 'Jobs', 'CronJobs'],
  },
  { group: 'Network', sections: ['Services', 'Ingresses', 'NetworkPolicies'] },
  { group: 'Config', sections: ['ConfigMaps', 'Secrets'] },
  { group: 'Storage', sections: ['PersistentVolumeClaims', 'PersistentVolumes', 'StorageClasses'] },
  { group: 'GitOps', sections: ['Flux'] },
]

export function paneTitle(pane: PaneState): string {
  switch (pane.kind) {
    case 'nav':
      return 'Navigator'
    case 'list':
      return pane.route?.section ?? 'Resources'
    case 'inspector':
      return pane.selection ?? 'Inspector'
    case 'tool':
      return pane.tool ? TOOL_LABEL[pane.tool] : 'Tool'
  }
}
