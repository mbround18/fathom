/**
 * Port Forwards tool panel (009 US2/US4): the live list of active/inactive forwards, reachable
 * from the desktop navigator (`network/portforwards` list route) and the phone/tablet `Forwards`
 * stack tab (`kind: 'tool', tool: 'portforwards'`). Per data-model.md, each row is computed here
 * by merging the backend's `PortForward` summary with `tunnels[pf.id]` (live status from
 * `PortForwardTunnel` stream events) and `portForwardOrigins[pf.id]` (decorative Service-origin
 * tag) — none of these are stored pre-merged.
 */
import { useEffect, useState } from 'react'
import { toast } from 'sonner'

import { ConfirmDialog } from '@/components/ConfirmDialog'
import { StatusBadge } from '@/components/StatusBadge'
import { Button, Checkbox } from '@companion/ui'
import { Gated } from '@/lib/capabilities'
import { SECTION } from '@/protocol/commands'
import type { PortForward } from '@/protocol/types'
import { listKey, useStore } from '@/store'

import { Empty } from '../shell/PaneStates'
import type { ForwardTarget } from './PortForwardStartDialog'
import { PortForwardStartDialog } from './PortForwardStartDialog'

interface PortForwardRow {
  pf: PortForward
  status: PortForward['status']
  localEndpoint: string | null
  service: string | undefined
}

export function PortForwardsPanel() {
  const send = useStore((s) => s.send)
  const list = useStore((s) => s.lists[listKey(null, SECTION.portForwards, 'Global', null)])
  const tunnels = useStore((s) => s.tunnels)
  const origins = useStore((s) => s.portForwardOrigins)

  useEffect(() => {
    send({ type: 'RefreshPortForwards' })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const [stopTarget, setStopTarget] = useState<PortForward | undefined>(undefined)
  const [removeTarget, setRemoveTarget] = useState<PortForward | undefined>(undefined)
  const [restartTarget, setRestartTarget] = useState<ForwardTarget | undefined>(undefined)
  const [restartOpen, setRestartOpen] = useState(false)

  const pfs = (list?.rows as PortForward[] | undefined) ?? []
  const rows: PortForwardRow[] = pfs.map((pf) => {
    const tunnel = tunnels[pf.id]
    return {
      pf,
      status: tunnel?.status ?? pf.status,
      localEndpoint: tunnel?.localEndpoint ?? null,
      service: origins[pf.id]?.service,
    }
  })

  if (rows.length === 0) {
    return (
      <Empty
        title="No active port forwards"
        detail="Start one from a Pod or Service inspector's Forward tab."
      />
    )
  }

  return (
    <div className="flex h-full min-h-0 flex-col overflow-auto" data-testid="port-forward-list">
      <ul className="flex flex-col">
        {rows.map((row) => (
          <PortForwardRowItem
            key={row.pf.id}
            row={row}
            onStop={() => {
              setStopTarget(row.pf)
            }}
            onRemove={() => {
              setRemoveTarget(row.pf)
            }}
            onRestart={() => {
              setRestartTarget({
                namespace: row.pf.namespace,
                pod: row.pf.pod,
                container: row.pf.container ?? undefined,
                remotePort: row.pf.remote_port,
                suggestedLocalPort: row.pf.local_port,
              })
              setRestartOpen(true)
            }}
          />
        ))}
      </ul>

      <ConfirmDialog
        open={removeTarget !== undefined}
        title="Remove port forward"
        items={removeTarget ? [{ namespace: removeTarget.namespace, name: removeTarget.pod }] : []}
        requireTypedCount={false}
        confirmLabel="Remove forward"
        detail={
          removeTarget ? (
            <span data-testid="port-forward-remove-detail">
              Removes the saved forward from local port {removeTarget.local_port} to{' '}
              {removeTarget.namespace}/{removeTarget.pod}:{removeTarget.remote_port}.
            </span>
          ) : undefined
        }
        onConfirm={() => {
          if (!removeTarget) return
          send({ type: 'DeletePortForward', data: { tunnel_id: removeTarget.id } })
          setRemoveTarget(undefined)
        }}
        onCancel={() => {
          setRemoveTarget(undefined)
        }}
      />

      <ConfirmDialog
        open={stopTarget !== undefined}
        title="Stop port forward"
        items={stopTarget ? [{ namespace: stopTarget.namespace, name: stopTarget.pod }] : []}
        requireTypedCount={false}
        confirmLabel="Stop forward"
        detail={
          stopTarget ? (
            <span data-testid="port-forward-stop-detail">
              Stops forwarding local port {stopTarget.local_port} to {stopTarget.namespace}/
              {stopTarget.pod}:{stopTarget.remote_port}.
            </span>
          ) : undefined
        }
        onConfirm={() => {
          if (!stopTarget) return
          send({ type: 'StopPortForward', data: { tunnel_id: stopTarget.id } })
          setStopTarget(undefined)
        }}
        onCancel={() => {
          setStopTarget(undefined)
        }}
      />

      <PortForwardStartDialog
        target={restartTarget}
        open={restartOpen}
        onOpenChange={setRestartOpen}
      />
    </div>
  )
}

function PortForwardRowItem({
  row,
  onStop,
  onRemove,
  onRestart,
}: {
  row: PortForwardRow
  onStop: () => void
  onRemove: () => void
  onRestart: () => void
}) {
  const send = useStore((s) => s.send)
  const { pf, status, localEndpoint, service } = row
  const canStop = status === 'Active' || status === 'Connecting'
  const canRestart = status === 'Error' || status === 'Closed'
  const canCopy = status === 'Active'

  const copyAddress = () => {
    const address = `localhost:${String(pf.local_port)}`
    void navigator.clipboard.writeText(address).then(() => {
      toast.success(`Copied ${address}`, { testId: 'port-forward-copy-toast' })
    })
  }

  return (
    <li
      data-testid="port-forward-row"
      data-tunnel-id={pf.id}
      className="flex flex-wrap items-center justify-between gap-2 border-b border-border px-3 py-2 font-mono text-xs last:border-b-0"
    >
      <div className="flex min-w-0 flex-col gap-0.5">
        <span className="flex items-center gap-2">
          <span data-testid="port-forward-local-port">{pf.local_port}</span>
          <span aria-hidden="true" className="text-muted-foreground">
            →
          </span>
          <span data-testid="port-forward-target" className="truncate">
            {pf.namespace}/{pf.pod}:{pf.remote_port}
          </span>
          {service && (
            <span
              data-testid="port-forward-origin-service"
              className="rounded-sm bg-muted px-1 py-0.5 text-xs text-muted-foreground uppercase"
            >
              via {service}
            </span>
          )}
        </span>
        <span className="flex items-center gap-2">
          <StatusBadge status={status} />
          {localEndpoint && <span className="text-muted-foreground">{localEndpoint}</span>}
        </span>
      </div>

      <div className="flex items-center gap-2">
        <label
          htmlFor={`port-forward-auto-start-${pf.id}`}
          className="flex items-center gap-1.5 text-xs text-muted-foreground uppercase"
        >
          <Checkbox
            id={`port-forward-auto-start-${pf.id}`}
            data-testid="port-forward-auto-start"
            checked={pf.auto_start}
            onCheckedChange={(checked) => {
              send({
                type: 'UpdatePortForward',
                data: {
                  tunnel_id: pf.id,
                  local_port: pf.local_port,
                  remote_port: pf.remote_port,
                  auto_start: checked === true,
                },
              })
            }}
          />
          Auto-start
        </label>
        <Button
          type="button"
          size="sm"
          variant="outline"
          data-testid="port-forward-copy-address"
          disabled={!canCopy}
          onClick={copyAddress}
        >
          Copy
        </Button>
        {canStop && (
          <Button type="button" size="sm" data-testid="port-forward-stop" onClick={onStop}>
            Stop
          </Button>
        )}
        {canRestart && (
          <Button
            type="button"
            size="sm"
            variant="outline"
            data-testid="port-forward-delete"
            onClick={onRemove}
          >
            Remove
          </Button>
        )}
        {canRestart && (
          <Gated verb="create" resource="pods" subresource="portforward" namespace={pf.namespace}>
            <Button type="button" size="sm" data-testid="port-forward-restart" onClick={onRestart}>
              Restart
            </Button>
          </Gated>
        )}
      </div>
    </li>
  )
}
