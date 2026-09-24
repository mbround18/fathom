/** 034-simultaneous-federation: choose which contexts stay live at once and see one aggregate
 * overview across them. Lives inside the Fleet dialog. Backend-owned state; this view only
 * sends `FederateContext`/`UnfederateContext` and renders what the backend reports. */
import { useEffect } from 'react'

import { useStore } from '@/store'

import { FederationRow } from './FederationRow'
import { FederationToggle } from './FederationToggle'

export function FederationView() {
  const contexts = useStore((s) => s.session.contexts)
  const fed = useStore((s) => s.session.federation)
  const federate = useStore((s) => s.federateContext)
  const unfederate = useStore((s) => s.unfederateContext)
  const requestSnapshot = useStore((s) => s.requestFederationSnapshot)

  useEffect(() => {
    requestSnapshot()
  }, [requestSnapshot])

  if (contexts.length === 0 && fed.federated.length === 0) return null

  const onChange = (context: string, on: boolean) => (on ? federate(context) : unfederate(context))
  // Federated contexts missing from the kubeconfig still get a row so they can be removed.
  const all = Array.from(new Set([...contexts, ...fed.federated])).sort()
  const combined = fed.combined

  return (
    <section data-testid="federation-view" className="flex flex-col gap-2 p-2">
      <h3 className="text-sm font-medium">Live federation</h3>
      <p className="text-xs text-muted-foreground">
        Keep contexts connected simultaneously. Only contexts switched on are watched.
      </p>
      {fed.overCap && (
        <p
          data-testid="federation-over-cap"
          role="status"
          className="rounded border border-amber-500/60 p-2 text-xs text-amber-400"
        >
          {fed.federated.length} contexts are live, above the recommended {fed.softCap}. Nothing was
          disconnected; consider turning some off to reduce load.
        </p>
      )}
      {combined && fed.federated.length > 0 && (
        <dl
          data-testid="federation-combined"
          className="grid grid-cols-2 gap-2 rounded-md border border-border/60 p-3 text-xs text-muted-foreground sm:grid-cols-5"
        >
          <div>
            <dt>Contexts live</dt>
            <dd data-testid="federation-combined-contexts" className="text-foreground">
              {combined.contributing_contexts}/{combined.total_contexts}
            </dd>
          </div>
          <div>
            <dt>Nodes</dt>
            <dd className="text-foreground">
              {combined.nodes_ready}/{combined.nodes_total}
            </dd>
          </div>
          <div>
            <dt>Pods running</dt>
            <dd className="text-foreground">{combined.pods_running}</dd>
          </div>
          <div>
            <dt>Unhealthy pods</dt>
            <dd data-testid="federation-combined-unhealthy" className="text-foreground">
              {combined.pods_unhealthy}
            </dd>
          </div>
          <div>
            <dt>Warnings (1h)</dt>
            <dd className="text-foreground">{combined.warning_events_1h}</dd>
          </div>
        </dl>
      )}
      <div className="grid grid-cols-1 gap-2 lg:grid-cols-2">
        {all.map((name) => {
          const rollup = fed.rollups[name]
          const on = fed.federated.includes(name)
          const toggle = <FederationToggle context={name} federated={on} onChange={onChange} />
          if (on && rollup) return <FederationRow key={name} rollup={rollup} toggle={toggle} />
          return (
            <div
              key={name}
              data-testid="federation-off-row"
              data-context-name={name}
              className="flex min-h-[var(--touch-target)] items-center justify-between gap-2 rounded-md border border-border/40 px-3"
            >
              <span className="min-w-0 truncate font-mono text-sm text-muted-foreground">
                {name}
              </span>
              {toggle}
            </div>
          )
        })}
      </div>
    </section>
  )
}
