/**
 * Node Shell tab (037-node-shell): a privileged debug pod pinned to one node, attached as an
 * interactive terminal. Everything shown reflects `NodeShellStatus` events from the backend —
 * the UI never assumes the pod exists or is gone. Leaving the tab, exiting the shell, or
 * pressing Stop all send `StopNodeShell`; the backend owns the actual cleanup.
 *
 * Stand-ins for the unmerged 024/025 features (research.md R7): the existing `ConfirmDialog`
 * for the privileged-action confirmation and a local SelfSubjectAccessReview check.
 */
import { ShieldWarningIcon, StopIcon, TerminalWindowIcon, TrashIcon } from '@phosphor-icons/react'
import { useEffect, useRef, useState } from 'react'

import { ConfirmDialog } from '@/components/ConfirmDialog'
import { Button, Input } from '@companion/ui'
import type { NodeDetails, OrphanedNodeShellPod } from '@/protocol/types'
import { detailKey, useStore } from '@/store'

import { Loading } from '@companion/terminal'
import { isWindowsNode, phaseView, summarizeAccess, WINDOWS_REASON } from './nodeShellView'
import { TerminalView, type TerminalPhase } from './TerminalView'

const DEFAULT_NAMESPACE = 'default'

export function NodeShellPanel({ nodeName }: { nodeName: string }) {
  const send = useStore((s) => s.send)
  const context = useStore((s) => s.session.activeContext)
  const settings = useStore((s) => s.nodeShell.settings)
  const access = useStore((s) => s.nodeShell.access)
  const orphans = useStore((s) => s.nodeShell.orphans)
  const session = useStore((s) => s.nodeShell.session)
  const detailState = useStore((s) => s.details[detailKey(context, 'Node', null, nodeName)])
  const os =
    detailState?.eventType === 'NodeDetailsUpdated'
      ? (detailState.data as NodeDetails).os
      : undefined
  const windows = isWindowsNode(os)

  const namespace = settings?.namespace ?? DEFAULT_NAMESPACE
  const [confirmOpen, setConfirmOpen] = useState(false)
  const [override, setOverride] = useState<{ image?: string; deadlineMinutes?: string }>({})
  const [orphanTarget, setOrphanTarget] = useState<OrphanedNodeShellPod | undefined>(undefined)

  useEffect(() => {
    send({ type: 'GetNodeShellSettings' })
  }, [send])
  useEffect(() => {
    send({ type: 'CheckNodeShellAccess', data: { namespace } })
  }, [send, namespace])

  const mine = session?.node === nodeName ? session : undefined
  const view = mine ? phaseView(mine.phase, mine.leftover) : undefined

  // Leaving the tab ends the session: the latest session is read through a ref so the unmount
  // cleanup (which runs once) sees current state.
  const latest = useRef(mine)
  useEffect(() => {
    latest.current = mine
  })
  useEffect(
    () => () => {
      const s = latest.current
      if (s && s.phase !== 'Removed' && !(s.phase === 'Failed' && !s.leftover)) {
        send({ type: 'StopNodeShell', data: { session_id: s.sessionId, namespace: s.namespace } })
      }
    },
    [send],
  )

  if (settings === undefined) return <Loading label="Loading Node Shell" />

  const summary = summarizeAccess(access?.checks ?? [])
  const accessKnown = access !== undefined
  const image = override.image?.trim() || settings.image
  const deadlineSeconds =
    override.deadlineMinutes !== undefined && override.deadlineMinutes !== ''
      ? Math.round(Number(override.deadlineMinutes) * 60)
      : settings.deadline_seconds
  const deadlineValid = Number.isFinite(deadlineSeconds) && deadlineSeconds >= 300
  const inFlight =
    mine !== undefined && mine.phase !== 'Removed' && !(mine.phase === 'Failed' && !mine.leftover)
  const startBlocked = windows || !summary.allowed || !deadlineValid || inFlight

  const start = () => {
    setConfirmOpen(false)
    send({
      type: 'StartNodeShell',
      data: {
        node: nodeName,
        namespace,
        image,
        image_pull_secret: settings.image_pull_secret,
        deadline_seconds: deadlineSeconds,
      },
    })
  }

  const stop = () => {
    if (mine)
      send({
        type: 'StopNodeShell',
        data: { session_id: mine.sessionId, namespace: mine.namespace },
      })
  }

  const onTerminalPhase = (phase: TerminalPhase) => {
    // The shell exited (or failed to attach): end the session so the pod is deleted.
    if ((phase === 'ended' || phase === 'error') && mine) stop()
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 p-3" data-testid="node-shell-panel">
      {!inFlight && (
        <section aria-label="Start a node shell" className="flex flex-col gap-2">
          <p className="text-xs text-muted-foreground">
            Starts a privileged pod on <span className="font-mono">{nodeName}</span> with host
            namespaces and the node filesystem at <span className="font-mono">/host</span>, then
            attaches a terminal. The pod is deleted when you leave.
          </p>

          {windows && (
            <p
              role="status"
              data-testid="node-shell-disabled-reason"
              className="text-xs text-status-warn"
            >
              {WINDOWS_REASON}
            </p>
          )}
          {!windows && accessKnown && !summary.allowed && (
            <div role="alert" data-testid="node-shell-denied" className="text-xs text-status-warn">
              <p className="flex items-center gap-1 font-semibold">
                <ShieldWarningIcon aria-hidden="true" className="size-4" />
                Node Shell is unavailable: missing permissions in {namespace}
              </p>
              <ul className="ml-5 list-disc font-mono">
                {summary.problems.map((p) => (
                  <li key={p}>{p}</li>
                ))}
              </ul>
            </div>
          )}

          <details className="text-xs">
            <summary className="cursor-pointer">Session options</summary>
            <div className="mt-2 flex flex-col gap-2">
              <div className="flex flex-col gap-1">
                <label htmlFor="node-shell-image">Image</label>
                <Input
                  id="node-shell-image"
                  data-testid="node-shell-image"
                  value={override.image ?? settings.image}
                  onChange={(e) => setOverride((o) => ({ ...o, image: e.target.value }))}
                  className="font-mono"
                />
              </div>
              <div className="flex flex-col gap-1">
                <label htmlFor="node-shell-deadline">Auto-delete after (minutes)</label>
                <Input
                  id="node-shell-deadline"
                  data-testid="node-shell-deadline"
                  inputMode="numeric"
                  value={
                    override.deadlineMinutes ?? String(Math.round(settings.deadline_seconds / 60))
                  }
                  onChange={(e) => setOverride((o) => ({ ...o, deadlineMinutes: e.target.value }))}
                  className="font-mono"
                />
              </div>
              {!deadlineValid && (
                <p role="alert" className="text-status-warn">
                  The deadline must be at least 5 minutes.
                </p>
              )}
            </div>
          </details>

          <div>
            <Button
              type="button"
              data-testid="node-shell-start"
              disabled={startBlocked}
              title={windows ? WINDOWS_REASON : undefined}
              onClick={() => setConfirmOpen(true)}
              className="h-[max(2rem,var(--touch-target))]"
            >
              <TerminalWindowIcon aria-hidden="true" className="size-4" />
              Open node shell
            </Button>
          </div>
        </section>
      )}

      {mine && view && (
        <section aria-label="Node shell session" className="flex min-h-0 flex-1 flex-col gap-2">
          <div
            role="status"
            data-testid="node-shell-status"
            data-phase={mine.phase}
            data-tone={view.tone}
            className="flex flex-wrap items-center gap-2 text-xs"
          >
            <span className="font-semibold">{view.label}</span>
            <span className="font-mono text-muted-foreground">
              {mine.namespace}/{mine.pod}
            </span>
            {view.canStop && (
              <Button
                type="button"
                variant="outline"
                size="sm"
                data-testid="node-shell-stop"
                onClick={stop}
                className="ml-auto"
              >
                <StopIcon aria-hidden="true" className="size-4" />
                {mine.leftover
                  ? 'Retry delete'
                  : mine.phase === 'Running' || mine.phase === 'Attached'
                    ? 'Stop'
                    : 'Cancel'}
              </Button>
            )}
          </div>
          {/* FR-010: conditions, waiting reasons and API errors are shown verbatim. */}
          {mine.detail && (
            <p
              data-testid="node-shell-detail"
              className="font-mono text-xs break-words text-status-warn"
            >
              {mine.detail}
            </p>
          )}
          {mine.leftover && (
            <p role="alert" data-testid="node-shell-leftover" className="text-xs text-status-warn">
              The debug pod{' '}
              <span className="font-mono">
                {mine.namespace}/{mine.pod}
              </span>{' '}
              still exists on the cluster. Retry the delete, or remove it with{' '}
              <span className="font-mono">
                kubectl -n {mine.namespace} delete pod {mine.pod}
              </span>
              .
            </p>
          )}
          {view.showTerminal && (
            <div className="flex h-72 min-h-0 flex-1 flex-col border border-border">
              <TerminalView
                key={mine.sessionId}
                namespace={mine.namespace}
                name={mine.pod}
                container="shell"
                testId="node-shell-xterm"
                onPhase={onTerminalPhase}
              />
            </div>
          )}
        </section>
      )}

      <section
        aria-label="Leftover node shell pods"
        className="flex flex-col gap-2 border-t border-border pt-2"
      >
        <div className="flex items-center gap-2">
          <h3 className="font-display text-xs tracking-wider uppercase">Leftover pods</h3>
          <Button
            type="button"
            variant="outline"
            size="sm"
            data-testid="node-shell-find-orphans"
            onClick={() => send({ type: 'ListNodeShellOrphans', data: { namespace: null } })}
            className="ml-auto"
          >
            Find leftover pods
          </Button>
        </div>
        {orphans !== undefined && orphans.length === 0 && (
          <p data-testid="node-shell-no-orphans" className="text-xs text-muted-foreground">
            No leftover node shell pods found.
          </p>
        )}
        {orphans?.map((o) => (
          <div
            key={`${o.namespace}/${o.name}`}
            data-testid="node-shell-orphan"
            className="flex items-center gap-2 text-xs"
          >
            <span className="min-w-0 flex-1 font-mono break-all">
              {o.namespace}/{o.name}
              {o.node ? ` on ${o.node}` : ''}
              {o.phase ? ` (${o.phase})` : ''}
            </span>
            <Button
              type="button"
              variant="outline"
              size="sm"
              data-testid="node-shell-orphan-delete"
              onClick={() => setOrphanTarget(o)}
            >
              <TrashIcon aria-hidden="true" className="size-4" />
              Delete
            </Button>
          </div>
        ))}
      </section>

      <ConfirmDialog
        open={confirmOpen}
        title="Open node shell"
        // Two entries, but one action: the node being opened and the pod that will be created
        // for it. `items` is a manifest here, not a count of objects being mutated, so the typed
        // count that guards bulk actions would ask the operator to type "2" about nothing.
        requireTypedCount={false}
        items={[
          { namespace: null, name: `node ${nodeName}` },
          { namespace, name: 'fathom-node-shell-* (created for this session)' },
        ]}
        detail={
          <>
            Image <span className="font-mono">{image}</span>. The pod runs privileged with hostPID,
            hostNetwork and hostIPC, and mounts the node root filesystem at{' '}
            <span className="font-mono">/host</span>. It is deleted when you leave, and Kubernetes
            deletes it after {Math.round(deadlineSeconds / 60)} minutes regardless.
          </>
        }
        confirmLabel="Open node shell"
        destructive
        onConfirm={start}
        onCancel={() => setConfirmOpen(false)}
      />
      <ConfirmDialog
        open={orphanTarget !== undefined}
        title="Delete leftover pod"
        items={orphanTarget ? [{ namespace: orphanTarget.namespace, name: orphanTarget.name }] : []}
        confirmLabel="Delete pod"
        destructive
        onConfirm={() => {
          if (orphanTarget) {
            send({
              type: 'DeleteNodeShellOrphans',
              data: { pods: [{ namespace: orphanTarget.namespace, name: orphanTarget.name }] },
            })
          }
          setOrphanTarget(undefined)
        }}
        onCancel={() => setOrphanTarget(undefined)}
      />
    </div>
  )
}
