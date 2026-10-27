/**
 * Forward-start confirmation dialog (009, US1/US3): shared by the Pod/Service Inspector "Forward"
 * tab (`PortForwardActionsPanel`) and the forwards list's "Restart" action
 * (`PortForwardsPanel`). Implements the `idle → confirming → submitting → closed|error` state
 * machine from data-model.md, reusing the shared `ConfirmDialog` (constitution Principle I) with
 * the port picker rendered in its `detail` slot.
 */
import { useEffect, useState } from 'react'
import { toast } from 'sonner'

import { ConfirmDialog } from '@/components/ConfirmDialog'
import { useStore } from '@/store'
import { waitForCommandResult } from '@/store/actions'

import { PortForwardPortField } from './PortForwardPortField'

/** What/where a forward originates from — not a new backend concept (spec.md Key Entities). */
export interface ForwardTarget {
  namespace: string
  pod: string
  container?: string | undefined
  /** The target/remote port to forward to. */
  remotePort: number
  /** Set when this target was resolved from a Service inspector (US1 Scenario 2). */
  service?: string
  /** Seeds the suggested local port; defaults to `remotePort` (US3 Scenario 1) or, for a
   * restart, the forward's previously-chosen `local_port` (data-model.md "Restart flow"). */
  suggestedLocalPort?: number
}

const TOAST_TEST_ID = 'port-forward-start-toast'

export function PortForwardStartDialog({
  target,
  open,
  onOpenChange,
}: {
  target: ForwardTarget | undefined
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const send = useStore((s) => s.send)
  const request = useStore((s) => s.request)
  const recordPortForwardOrigin = useStore((s) => s.recordPortForwardOrigin)

  const [localPort, setLocalPort] = useState(0)
  const [conflict, setConflict] = useState(false)
  const [submitting, setSubmitting] = useState(false)
  const [error, setError] = useState<string | undefined>(undefined)

  // Reset per-target state on open, derived during render (react.dev "Adjusting state when a
  // prop changes") rather than an effect, keyed by the target's identity so it fires exactly
  // once per open per target (including a restart re-opening on a different row).
  const openKey =
    open && target
      ? `${target.namespace}|${target.pod}|${target.container ?? ''}|${String(target.remotePort)}`
      : ''
  const [prevOpenKey, setPrevOpenKey] = useState('')
  if (openKey !== prevOpenKey) {
    setPrevOpenKey(openKey)
    if (openKey !== '' && target) {
      setLocalPort(target.suggestedLocalPort ?? target.remotePort)
      setSubmitting(false)
      setError(undefined)
    }
  }

  // US3 Scenario 1: the suggested port has "already been checked for availability" by the time
  // the dialog is shown — this effect only dispatches the check, it never sets local state.
  useEffect(() => {
    if (!open || !target) return
    send({
      type: 'CheckPortAvailability',
      data: { port: target.suggestedLocalPort ?? target.remotePort },
    })
  }, [open, target, send])

  if (!target) return null

  const onConfirm = () => {
    // FR-006 / US3 Scenario 2: an unresolved conflict blocks dispatch rather than allowing a
    // blind attempt. A late backend bind failure (race, US3 Scenario 4) still surfaces verbatim
    // via `CommandFinished`'s `error`, same as any other start failure (FR-009).
    if (conflict) return
    setSubmitting(true)
    setError(undefined)
    void (async () => {
      let requestId: number
      try {
        requestId = await request({
          type: 'StartPortForward',
          data: {
            namespace: target.namespace,
            pod: target.pod,
            local_port: localPort,
            remote_port: target.remotePort,
            container: target.container ?? null,
          },
        })
      } catch (err) {
        setSubmitting(false)
        setError(err instanceof Error ? err.message : String(err))
        return
      }
      const result = await waitForCommandResult(requestId)
      if (result === 'timeout') {
        setSubmitting(false)
        setError('No response from backend.')
        return
      }
      if (!result.ok) {
        setSubmitting(false)
        setError(result.error ?? 'Failed to start the port forward.')
        return
      }
      if (target.service) {
        recordPortForwardOrigin({
          namespace: target.namespace,
          pod: target.pod,
          localPort: localPort,
          remotePort: target.remotePort,
          service: target.service,
        })
      }
      toast.success(
        `Starting forward ${target.namespace}/${target.pod}:${String(target.remotePort)}`,
        {
          testId: TOAST_TEST_ID,
        },
      )
      setSubmitting(false)
      onOpenChange(false)
    })()
  }

  return (
    <ConfirmDialog
      open={open}
      title="Start port forward"
      items={[{ namespace: target.namespace, name: target.pod }]}
      requireTypedCount={false}
      confirmLabel={submitting ? 'Starting…' : 'Start forward'}
      onConfirm={onConfirm}
      onCancel={() => {
        onOpenChange(false)
      }}
      detail={
        <div className="flex flex-col gap-3" data-testid="port-forward-start-dialog">
          <p>
            Forwards {target.container ? `container ${target.container}` : 'pod'} port{' '}
            <span className="font-mono">{target.remotePort}</span> in{' '}
            <span className="font-mono">
              {target.namespace}/{target.pod}
            </span>
            {target.service && (
              <>
                {' '}
                (resolved from Service <span className="font-mono">{target.service}</span>)
              </>
            )}{' '}
            to a local port.
          </p>
          <PortForwardPortField
            id="port-forward-local-port-input"
            value={localPort}
            onChange={setLocalPort}
            onConflictChange={setConflict}
          />
          {conflict && (
            <p data-testid="port-forward-confirm-blocked" className="text-xs text-status-err">
              Choose a different local port before starting the forward.
            </p>
          )}
          {error && (
            <p data-testid="port-forward-start-error" className="text-xs text-status-err">
              {error}
            </p>
          )}
        </div>
      }
    />
  )
}
