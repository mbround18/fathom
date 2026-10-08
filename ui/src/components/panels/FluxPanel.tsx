/**
 * Flux GitOps Toolkit tool pane: renders `store.flux.overview` (SystemEvent::FluxOverviewUpdated)
 * through `IntegrationSummary`. State arrives from a 20s background scan started on connect
 * (see backend `sync.rs::ensure_flux_poll`); opening the pane also sends `RefreshFlux` so it
 * doesn't wait out the rest of that interval.
 */
import { GitBranchIcon } from '@phosphor-icons/react'
import { useEffect } from 'react'

import type { FluxReadyState, FluxResourceSummary } from '@/protocol/types'
import { useStore } from '@/store'
import { FLUX_RESOURCE_SECTION } from '@/store/keys'

import { IntegrationSummary, type SummaryRow, type SummaryTile } from './IntegrationSummary'
import { MetricsUnavailable } from '../shell/PaneStates'

const READY_TONE: Record<FluxReadyState, SummaryRow['tone']> = {
  Ready: 'ok',
  Reconciling: 'warn',
  Suspended: 'muted',
  Failed: 'err',
  Unknown: 'muted',
}

function countReady(items: FluxResourceSummary[]): string {
  const ready = items.filter((i) => i.ready === 'Ready').length
  return `${String(ready)}/${String(items.length)}`
}

export function FluxPanel() {
  const overview = useStore((s) => s.flux.overview)
  const refreshFlux = useStore((s) => s.refreshFlux)
  const openInspector = useStore((s) => s.openInspector)

  useEffect(() => {
    refreshFlux()
  }, [refreshFlux])

  if (!overview) {
    return <MetricsUnavailable title="Scanning" detail="Waiting for the Flux scan to complete." />
  }
  if (!overview.installed) {
    return (
      <MetricsUnavailable
        title="Not installed"
        detail="Flux GitOps Toolkit CRDs were not found in this cluster. Install Flux (flux install) to see sources, Kustomizations and HelmReleases here."
      />
    )
  }

  const tiles: SummaryTile[] = [
    { label: 'Sources', value: countReady(overview.sources), tone: 'ok' },
    { label: 'Kustomizations', value: countReady(overview.kustomizations), tone: 'ok' },
    { label: 'HelmReleases', value: countReady(overview.helmreleases), tone: 'ok' },
  ]

  // 049 (US3): every resource is reachable from the pane, sources → Kustomizations → HelmReleases
  // with the ones needing attention first in each group; a click opens it in the inspector,
  // where Suspend / Resume / Reconcile live.
  const groups = [overview.sources, overview.kustomizations, overview.helmreleases]
  const rows: SummaryRow[] = groups
    .flatMap((g) => [
      ...g.filter((r) => r.ready !== 'Ready'),
      ...g.filter((r) => r.ready === 'Ready'),
    ])
    .map((r) => ({
      key: `${r.kind}/${r.namespace}/${r.name}`,
      primary: `${r.kind} ${r.namespace}/${r.name}`,
      secondary: r.suspended ? `Suspended · ${r.message || r.ready}` : r.message || r.ready,
      tone: READY_TONE[r.ready],
      testId: 'flux-row',
      onSelect: () =>
        openInspector({
          section: FLUX_RESOURCE_SECTION,
          kind: r.kind,
          namespace: r.namespace,
          name: r.name,
        }),
    }))

  return (
    <IntegrationSummary
      icon={GitBranchIcon}
      title="Flux"
      tiles={tiles}
      rows={rows}
      emptyLabel="No Flux sources, kustomizations or releases found."
    />
  )
}
