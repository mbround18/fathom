/**
 * Hubble/Cilium tool pane: renders `store.hubble.summary`
 * (SystemEvent::HubbleTrafficUpdated) through `IntegrationSummary`. State arrives from a
 * background poll of Hubble Relay's `Observer.GetFlows` (see backend `hubble.rs`) every 20s;
 * opening the pane also sends `RefreshHubble` so it doesn't wait out the rest of that interval.
 */
import { NetworkIcon } from '@phosphor-icons/react'
import { useEffect } from 'react'

import { useStore } from '@/store'

import { IntegrationSummary, type SummaryRow, type SummaryTile } from './IntegrationSummary'
import { MetricsUnavailable } from '../shell/PaneStates'

export function HubblePanel() {
  const summary = useStore((s) => s.hubble.summary)
  const refreshHubble = useStore((s) => s.refreshHubble)

  useEffect(() => {
    refreshHubble()
  }, [refreshHubble])

  if (!summary) {
    return (
      <MetricsUnavailable
        title="Scanning"
        detail="Waiting for the Hubble traffic poll to complete."
      />
    )
  }
  if (!summary.installed) {
    return (
      <MetricsUnavailable
        title="Not installed"
        detail="No hubble-relay Service found — Cilium/Hubble is not installed in this cluster. Install Cilium with Hubble Relay enabled to see network flows here."
      />
    )
  }

  const tiles: SummaryTile[] = [
    { label: 'Flows sampled', value: String(summary.total_flows), tone: 'muted' },
    { label: 'Forwarded', value: String(summary.forwarded), tone: 'ok' },
    { label: 'Dropped', value: String(summary.dropped), tone: summary.dropped > 0 ? 'err' : 'ok' },
  ]

  const pairRows: SummaryRow[] = summary.top_pairs.map((p) => ({
    key: `${p.source_namespace}->${p.dest_namespace}`,
    primary: `${p.source_namespace || '(host)'} → ${p.dest_namespace || '(host)'}`,
    secondary: `${String(p.count)} flow${p.count === 1 ? '' : 's'}`,
    tone: 'ok',
  }))
  const reasonRows: SummaryRow[] = summary.top_drop_reasons.map((r) => ({
    key: r.reason,
    primary: r.reason,
    secondary: `${String(r.count)} dropped flow${r.count === 1 ? '' : 's'}`,
    tone: 'err',
  }))

  return (
    <IntegrationSummary
      icon={NetworkIcon}
      title="Hubble"
      tiles={tiles}
      rows={[...pairRows, ...reasonRows]}
      emptyLabel="No traffic observed in the last sample window."
    />
  )
}
