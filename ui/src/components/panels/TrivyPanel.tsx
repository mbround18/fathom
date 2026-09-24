/**
 * Cluster-wide Trivy Operator vulnerability tool pane (021-trivy-vulnerability-pane, US2):
 * renders `store.trivy.overview` ranked worst-first (the backend pre-sorts). Selecting a row
 * opens that workload in the Inspector.
 */
import { useEffect } from 'react'

import type { SectionKey, WorkloadVulnSummary } from '@/protocol/types'
import { useStore } from '@/store'

import { MetricsUnavailable } from '../shell/PaneStates'

const KIND_SECTION: Record<string, SectionKey> = {
  Deployment: 'workloads/deployments',
  StatefulSet: 'workloads/statefulsets',
  DaemonSet: 'workloads/daemonsets',
  ReplicaSet: 'workloads/replicasets',
  Job: 'workloads/jobs',
  CronJob: 'workloads/cronjobs',
  Pod: 'workloads/pods',
}

export function TrivyPanel() {
  const overview = useStore((s) => s.trivy.overview)
  const refresh = useStore((s) => s.refreshTrivyOverview)
  const openInspector = useStore((s) => s.openInspector)

  useEffect(() => {
    refresh()
  }, [refresh])

  if (!overview) {
    return <MetricsUnavailable detail="Waiting for the Trivy scan to complete." />
  }
  if (!overview.installed) {
    return <MetricsUnavailable detail="Trivy Operator CRDs were not found in this cluster." />
  }
  if (overview.forbidden) {
    return (
      <MetricsUnavailable detail="Permission denied reading Trivy VulnerabilityReports." />
    )
  }
  if (overview.workloads.length === 0) {
    return (
      <p className="p-3 text-xs text-muted-foreground" data-testid="trivy-empty">
        No workloads scanned yet.
      </p>
    )
  }

  const open = (w: WorkloadVulnSummary) =>
    openInspector({
      section: KIND_SECTION[w.kind] ?? 'workloads/pods',
      kind: w.kind,
      namespace: w.namespace,
      name: w.name,
    })

  return (
    <div className="flex h-full min-h-0 flex-col gap-1 overflow-auto p-3" data-testid="trivy-panel">
      {overview.workloads.map((w) => (
        <button
          key={`${w.kind}/${w.namespace}/${w.name}`}
          type="button"
          data-testid="trivy-workload-row"
          onClick={() => {
            open(w)
          }}
          className="flex items-center gap-3 rounded-sm border border-border/60 bg-card/60 px-2 py-1.5 text-left font-mono text-xs"
        >
          <span className="min-w-0 flex-1 truncate">
            {w.kind} {w.namespace}/{w.name}
          </span>
          <span className="text-status-err">C {String(w.summary.critical)}</span>
          <span className="text-status-warn">H {String(w.summary.high)}</span>
          <span>M {String(w.summary.medium)}</span>
          <span className="text-muted-foreground">L {String(w.summary.low)}</span>
        </button>
      ))}
    </div>
  )
}
