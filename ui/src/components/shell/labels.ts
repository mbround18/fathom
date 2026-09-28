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

/**
 * 044 (T052a): the header of a cascade column that is still waiting. "Inspector" names the widget,
 * not the job, which tells the operator nothing about why the column is there. `step` is the
 * column's cascade number (`'1'` at the first cascade slot), so the first column names what a row
 * click will put in it and the rest name the follow that fills them.
 */
export function cascadeWaitingTitle(step: string): string {
  return step === '1' ? 'Selected item' : 'Next level'
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
