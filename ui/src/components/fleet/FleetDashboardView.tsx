/** 027-fleet-rollup-dashboard: read-only grid of every context's health. Backend pollers run
 * only while this view is mounted (Constitution V). */
import { useEffect, useState } from 'react'

import type { FleetContextSummary } from '@/protocol/types'
import { useStore } from '@/store'

import { FleetContextCard } from './FleetContextCard'

function pending(context: string): FleetContextSummary {
  return {
    context,
    health: 'Pending',
    error: null,
    plugin_error: null,
    node_ready_count: null,
    node_total_count: null,
    unhealthy_pod_count: null,
    cluster_version: null,
    flux: null,
    restricted: [],
    last_updated: '',
  }
}

export function FleetDashboardView({ onSelected }: { onSelected?: () => void }) {
  const contexts = useStore((s) => s.session.contexts)
  const summaries = useStore((s) => s.session.fleetSummaries)
  const active = useStore((s) => s.session.activeContext)
  const selectContext = useStore((s) => s.selectContext)
  const start = useStore((s) => s.startFleetPolling)
  const stop = useStore((s) => s.stopFleetPolling)

  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 5000)
    return () => clearInterval(id)
  }, [])

  useEffect(() => {
    start()
    return () => stop()
  }, [start, stop])

  if (contexts.length === 0) {
    return (
      <p data-testid="fleet-empty" className="p-4 text-sm text-muted-foreground">
        Connect a context to see fleet health.
      </p>
    )
  }

  return (
    <div
      data-testid="fleet-dashboard"
      className="grid grid-cols-1 gap-2 p-2 sm:grid-cols-2 lg:grid-cols-3"
    >
      {contexts.map((name) => (
        <FleetContextCard
          key={name}
          summary={summaries[name] ?? pending(name)}
          active={name === active}
          now={now}
          onSelect={(c) => {
            selectContext(c)
            onSelected?.()
          }}
        />
      ))}
    </div>
  )
}
