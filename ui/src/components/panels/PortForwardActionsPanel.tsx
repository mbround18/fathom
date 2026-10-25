/**
 * Pod/Service Inspector "Forward" tab (009, FR-001/FR-002): lists the target's declared ports
 * (each with a one-click "Forward" button) plus a manual-port fallback for the edge case where
 * none are declared, and opens `PortForwardStartDialog` per selection.
 *
 * For a Service target (US1 Scenario 2), resolving a backing pod reuses the existing
 * `RefreshRelatedResources` pattern (`src/backend/related.rs::related_for_service`) rather than
 * new resolution logic — the same command/event pair `pods.tsx`/`nodes.tsx` already send for
 * their own "Related" tabs.
 */
import { useEffect, useState } from 'react'

import { Button, Input } from '@companion/ui'
import { Gated } from '@/lib/capabilities'
import type { ContainerPort, RelatedResourceRef, ServicePort } from '@/protocol/types'
import { detailKey, useStore } from '@/store'

import { Empty } from '@companion/terminal'
import type { ForwardTarget } from './PortForwardStartDialog'
import { PortForwardStartDialog } from './PortForwardStartDialog'

export function PortForwardActionsPanel({
  kind,
  namespace,
  name,
  containerPorts,
  servicePorts,
}: {
  kind: 'Pod' | 'Service'
  namespace: string
  name: string
  /** Declared ports for a Pod target (`PodDetails.ports`), when `kind === 'Pod'`. */
  containerPorts?: readonly ContainerPort[]
  /** Declared ports for a Service target (`ServiceDetails.ports`), when `kind === 'Service'`. */
  servicePorts?: readonly ServicePort[]
}) {
  const [manualPort, setManualPort] = useState('')
  const [dialogTarget, setDialogTarget] = useState<ForwardTarget | undefined>(undefined)
  const [dialogOpen, setDialogOpen] = useState(false)

  const startPod = (remotePort: number, container?: string) => {
    setDialogTarget({ namespace, pod: name, container, remotePort })
    setDialogOpen(true)
  }

  const hasDeclaredPorts =
    kind === 'Pod' ? (containerPorts?.length ?? 0) > 0 : (servicePorts?.length ?? 0) > 0

  return (
    <div className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-3">
      <section className="flex flex-col gap-2">
        <h3 className="font-display text-xs font-semibold tracking-wider text-accent-neon uppercase">
          Forward
        </h3>
        {kind === 'Pod' ? (
          <PodPortList namespace={namespace} ports={containerPorts ?? []} onForward={startPod} />
        ) : (
          <ServicePortList
            namespace={namespace}
            name={name}
            ports={servicePorts ?? []}
            onForward={(remotePort, pod) => {
              setDialogTarget({ namespace, pod, remotePort, service: name })
              setDialogOpen(true)
            }}
          />
        )}
        {!hasDeclaredPorts && kind === 'Pod' && (
          <div className="flex items-end gap-2">
            <div className="flex flex-col gap-1">
              <label htmlFor="port-forward-manual-port" className="text-xs text-muted-foreground">
                Container port (none declared)
              </label>
              <Input
                id="port-forward-manual-port"
                data-testid="port-forward-manual-port"
                inputMode="numeric"
                autoComplete="off"
                value={manualPort}
                onChange={(e) => setManualPort(e.target.value)}
                className="w-32 font-mono"
              />
            </div>
            <Gated verb="create" resource="pods" subresource="portforward" namespace={namespace}>
              <Button
                type="button"
                size="sm"
                data-testid="port-forward-action-manual"
                disabled={!/^\d+$/.test(manualPort.trim())}
                onClick={() => {
                  startPod(Number.parseInt(manualPort, 10))
                }}
              >
                Forward
              </Button>
            </Gated>
          </div>
        )}
      </section>

      <PortForwardStartDialog
        target={dialogTarget}
        open={dialogOpen}
        onOpenChange={setDialogOpen}
      />
    </div>
  )
}

function PodPortList({
  namespace,
  ports,
  onForward,
}: {
  namespace: string
  ports: readonly ContainerPort[]
  onForward: (remotePort: number, container: string) => void
}) {
  if (ports.length === 0) {
    return (
      <Empty
        title="No declared ports"
        detail="This pod declares no container ports; use the manual port field below."
      />
    )
  }
  return (
    <ul className="flex flex-col gap-1">
      {ports.map((p) => (
        <li
          key={`${p.container}-${String(p.port)}`}
          className="flex items-center justify-between gap-2 border border-border px-2 py-1.5 font-mono text-xs"
        >
          <span>
            {p.name ?? p.container}: {p.port} ({p.protocol})
          </span>
          <Gated verb="create" resource="pods" subresource="portforward" namespace={namespace}>
            <Button
              type="button"
              size="sm"
              data-testid={`port-forward-action-${p.container}-${String(p.port)}`}
              onClick={() => {
                onForward(p.port, p.container)
              }}
            >
              Forward
            </Button>
          </Gated>
        </li>
      ))}
    </ul>
  )
}

/** Resolves one ready backing pod via `RefreshRelatedResources` (research.md), preferring a
 * `Running` one, and surfaces an explicit error if none exist/are ready (spec.md edge case). */
function ServicePortList({
  namespace,
  name,
  ports,
  onForward,
}: {
  namespace: string
  name: string
  ports: readonly ServicePort[]
  onForward: (remotePort: number, pod: string) => void
}) {
  const send = useStore((s) => s.send)
  const ctx = useStore((s) => s.session.activeContext)
  const relatedState = useStore(
    (s) => s.details[detailKey(ctx, 'RelatedResources', namespace, name)],
  )

  useEffect(() => {
    send({
      type: 'RefreshRelatedResources',
      data: { section: 'network/services', kind: 'Service', namespace, name },
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [namespace, name])

  const related =
    relatedState?.eventType === 'RelatedResourcesUpdated'
      ? (relatedState.data as { related: RelatedResourceRef[] }).related
      : undefined
  const pods = related?.filter((r) => r.kind === 'Pod') ?? []
  const readyPod = pods.find((r) => r.status_hint === 'Running') ?? pods[0]

  if (ports.length === 0) {
    return <Empty title="No declared ports" detail="This service declares no ports." />
  }
  if (related !== undefined && pods.length === 0) {
    return (
      <p data-testid="port-forward-service-no-pod" className="text-xs text-status-err">
        No backing pods found for this service.
      </p>
    )
  }
  if (related !== undefined && !readyPod) {
    return (
      <p data-testid="port-forward-service-no-pod" className="text-xs text-status-err">
        This service&apos;s backing pods are not yet ready.
      </p>
    )
  }

  return (
    <ul className="flex flex-col gap-1">
      {ports.map((p) => (
        <li
          key={`${p.name ?? ''}-${String(p.port)}`}
          className="flex items-center justify-between gap-2 border border-border px-2 py-1.5 font-mono text-xs"
        >
          <span>
            {p.name ?? '(unnamed)'}: {p.port} ({p.protocol})
          </span>
          <Gated verb="create" resource="pods" subresource="portforward" namespace={namespace}>
            <Button
              type="button"
              size="sm"
              data-testid={`port-forward-action-service-${String(p.port)}`}
              disabled={!readyPod}
              onClick={() => {
                if (readyPod) onForward(p.port, readyPod.name)
              }}
            >
              Forward
            </Button>
          </Gated>
        </li>
      ))}
    </ul>
  )
}
