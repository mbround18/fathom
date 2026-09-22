/**
 * Node actions tab (US2): cordon/uncordon/drain/taint, each behind the same single-click
 * `ConfirmDialog` every other mutating action uses (constitution I) — including drain, which
 * spec.md explicitly calls out as never requiring typed-count confirmation (it only kicks in
 * automatically above `ConfirmDialog`'s five-item threshold, and this dialog always names
 * exactly one node).
 *
 * Bypasses `ActionDialogHost`/`runAction` (data-model.md "Node Action" is its own,
 * frontend-only state shape, not a `ListRow` bulk action): drain and taint need extra input
 * (an ignore-daemonsets checkbox; a key/value/effect form) that `ActionInput` doesn't carry, and
 * a drain's confirmation kicks off a *streamed* result (`DrainProgress`/`NodeActionFinished`,
 * handled in `store/handlers/stream.ts`) rather than a single `CommandFinished`.
 */
import { ProhibitIcon, StopIcon, TagIcon, TrashIcon } from '@phosphor-icons/react'
import { useState } from 'react'
import { toast } from 'sonner'

import { ConfirmDialog } from '@/components/ConfirmDialog'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { SECTION } from '@/protocol/commands'
import { ACTION_TOAST_TEST_ID, waitForCommandResult } from '@/store/actions'
import { useStore, type NodeAction } from '@/store'

const ACTION_LABEL: Record<NodeAction['kind'], string> = {
  cordon: 'Cordon',
  uncordon: 'Uncordon',
  drain: 'Drain',
  'taint-add': 'Add taint to',
  'taint-remove': 'Remove taint from',
}

type PendingConfirm =
  | { kind: 'cordon' }
  | { kind: 'uncordon' }
  | { kind: 'drain'; ignoreDaemonsets: boolean; forceDeleteEmptyDir: boolean }
  | { kind: 'taint-add'; key: string; value: string; effect: string }
  | { kind: 'taint-remove'; key: string; value: string; effect: string }

const TAINT_EFFECTS = ['NoSchedule', 'PreferNoSchedule', 'NoExecute']

let toastSeq = 0

export function NodeActionsPanel({ nodeName }: { nodeName: string }) {
  const request = useStore((s) => s.request)
  const setNodeAction = useStore((s) => s.setNodeAction)
  const action = useStore((s) => s.nodeActions[nodeName])

  const [confirm, setConfirm] = useState<PendingConfirm | undefined>(undefined)
  const [ignoreDaemonsets, setIgnoreDaemonsets] = useState(true)
  const [forceDeleteEmptyDir, setForceDeleteEmptyDir] = useState(false)
  const [taintKey, setTaintKey] = useState('')
  const [taintValue, setTaintValue] = useState('')
  const [taintEffect, setTaintEffect] = useState(TAINT_EFFECTS[0] ?? 'NoSchedule')

  const busy = action?.status === 'in-progress' || action?.status === 'confirming'

  const runSimple = async (kind: NodeAction['kind'], send: () => Promise<number>) => {
    setNodeAction(nodeName, { nodeName, kind, status: 'in-progress' })
    const label = `${ACTION_LABEL[kind]} ${nodeName}`
    toastSeq += 1
    const toastId = `node-action-${nodeName}-${String(toastSeq)}`
    const base = { id: toastId, testId: ACTION_TOAST_TEST_ID }
    toast.loading(`${label}…`, base)
    let requestId: number
    try {
      requestId = await send()
    } catch (err) {
      const error = err instanceof Error ? err.message : String(err)
      setNodeAction(nodeName, { nodeName, kind, status: 'failed', error })
      toast.error(`✖ ${label} failed`, { ...base, description: error, duration: Infinity })
      return
    }
    setNodeAction(nodeName, {
      nodeName,
      kind,
      status: 'in-progress',
      requestId: String(requestId),
    })
    const result = await waitForCommandResult(requestId)
    if (result === 'timeout') {
      setNodeAction(nodeName, {
        nodeName,
        kind,
        status: 'failed',
        error: 'No response from backend.',
        requestId: String(requestId),
      })
      toast.error(`✖ No response from backend: ${label}`, base)
      return
    }
    if (result.ok) {
      setNodeAction(nodeName, {
        nodeName,
        kind,
        status: 'complete',
        requestId: String(requestId),
      })
      toast.success(`✓ ${label}`, base)
      useStore.getState().refreshSection(SECTION.nodes)
    } else {
      const error = result.error ?? 'The backend reported a failure with no message.'
      setNodeAction(nodeName, {
        nodeName,
        kind,
        status: 'failed',
        error,
        requestId: String(requestId),
      })
      toast.error(`✖ ${label} failed`, { ...base, description: error, duration: Infinity })
    }
  }

  const onConfirm = () => {
    if (!confirm) return
    const c = confirm
    setConfirm(undefined)
    switch (c.kind) {
      case 'cordon':
        void runSimple('cordon', () => request({ type: 'CordonNode', data: { name: nodeName } }))
        return
      case 'uncordon':
        void runSimple('uncordon', () =>
          request({ type: 'UncordonNode', data: { name: nodeName } }),
        )
        return
      case 'drain':
        void runDrain(c.ignoreDaemonsets, c.forceDeleteEmptyDir)
        return
      case 'taint-add':
        void runSimple('taint-add', () =>
          request({
            type: 'TaintNode',
            data: { name: nodeName, op: 'Add', key: c.key, value: c.value || null, effect: c.effect },
          }),
        )
        return
      case 'taint-remove':
        void runSimple('taint-remove', () =>
          request({
            type: 'TaintNode',
            data: {
              name: nodeName,
              op: 'Remove',
              key: c.key,
              value: c.value || null,
              effect: c.effect,
            },
          }),
        )
    }
  }

  const runDrain = async (ignoreDs: boolean, forceEmptyDir: boolean) => {
    setNodeAction(nodeName, { nodeName, kind: 'drain', status: 'in-progress' })
    try {
      const requestId = await request({
        type: 'DrainNode',
        data: {
          name: nodeName,
          ignore_daemonsets: ignoreDs,
          force_delete_empty_dir: forceEmptyDir,
        },
      })
      setNodeAction(nodeName, {
        nodeName,
        kind: 'drain',
        status: 'in-progress',
        requestId: String(requestId),
        drain: {
          totalPods: 0,
          evicted: [],
          blockedOn: [],
          ignoredDaemonSetPods: [],
          cancelled: false,
        },
      })
    } catch (err) {
      setNodeAction(nodeName, {
        nodeName,
        kind: 'drain',
        status: 'failed',
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  const cancelDrain = () => {
    if (!action?.requestId) return
    // Optimistic: the backend aborts the task and best-effort emits its own
    // `NodeActionFinished`, but the UI reflects "cancelled" immediately (spec.md US2).
    setNodeAction(nodeName, { ...action, status: 'cancelled' })
    useStore
      .getState()
      .send({ type: 'CancelDrainNode', data: { name: nodeName, request_id: action.requestId } })
  }

  const draining = action?.kind === 'drain' && action.status === 'in-progress'

  return (
    <div className="flex h-full min-h-0 flex-col gap-4 overflow-auto p-3">
      <section className="flex flex-col gap-2">
        <h3 className="font-display text-xs font-semibold tracking-wider text-accent-neon uppercase">
          Scheduling
        </h3>
        <div className="flex flex-wrap gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={busy}
            data-testid="node-action-cordon"
            onClick={() => setConfirm({ kind: 'cordon' })}
          >
            <ProhibitIcon data-icon="inline-start" /> Cordon
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={busy}
            data-testid="node-action-uncordon"
            onClick={() => setConfirm({ kind: 'uncordon' })}
          >
            <StopIcon data-icon="inline-start" /> Uncordon
          </Button>
        </div>
      </section>

      <section className="flex flex-col gap-2 border-t border-border pt-3">
        <h3 className="font-display text-xs font-semibold tracking-wider text-accent-neon uppercase">
          Drain
        </h3>
        <div className="flex items-center gap-2">
          <Checkbox
            id="ignore-daemonsets"
            checked={ignoreDaemonsets}
            onCheckedChange={(v) => setIgnoreDaemonsets(v === true)}
          />
          <label htmlFor="ignore-daemonsets" className="text-xs">
            Ignore DaemonSet-managed pods
          </label>
        </div>
        <div className="flex items-center gap-2">
          <Checkbox
            id="force-empty-dir"
            checked={forceDeleteEmptyDir}
            onCheckedChange={(v) => setForceDeleteEmptyDir(v === true)}
          />
          <label htmlFor="force-empty-dir" className="text-xs">
            Delete pods using emptyDir volumes
          </label>
        </div>
        <div className="flex gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={busy}
            data-testid="node-action-drain"
            onClick={() => setConfirm({ kind: 'drain', ignoreDaemonsets, forceDeleteEmptyDir })}
          >
            <TrashIcon data-icon="inline-start" /> Drain
          </Button>
          {draining && (
            <Button
              type="button"
              variant="destructive"
              size="sm"
              data-testid="node-action-cancel-drain"
              onClick={cancelDrain}
            >
              Cancel drain
            </Button>
          )}
        </div>
        {action?.kind === 'drain' && action.drain && (
          <DrainProgressView drain={action.drain} status={action.status} />
        )}
        {action?.kind === 'drain' && action.status === 'failed' && (
          <p data-testid="node-action-error" className="text-xs text-status-err">
            {action.error}
          </p>
        )}
      </section>

      <section className="flex flex-col gap-2 border-t border-border pt-3">
        <h3 className="font-display text-xs font-semibold tracking-wider text-accent-neon uppercase">
          Taints
        </h3>
        <div className="flex flex-wrap items-center gap-2">
          <Input
            data-testid="taint-key"
            placeholder="key"
            value={taintKey}
            onChange={(e) => setTaintKey(e.target.value)}
            className="h-8 w-32 font-mono"
          />
          <Input
            data-testid="taint-value"
            placeholder="value (optional)"
            value={taintValue}
            onChange={(e) => setTaintValue(e.target.value)}
            className="h-8 w-32 font-mono"
          />
          <select
            data-testid="taint-effect"
            value={taintEffect}
            onChange={(e) => setTaintEffect(e.target.value)}
            className="h-8 rounded-sm border border-border bg-background px-2 text-xs"
          >
            {TAINT_EFFECTS.map((e) => (
              <option key={e} value={e}>
                {e}
              </option>
            ))}
          </select>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={busy || !taintKey}
            data-testid="node-action-taint-add"
            onClick={() =>
              setConfirm({ kind: 'taint-add', key: taintKey, value: taintValue, effect: taintEffect })
            }
          >
            <TagIcon data-icon="inline-start" /> Add taint
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={busy || !taintKey}
            data-testid="node-action-taint-remove"
            onClick={() =>
              setConfirm({
                kind: 'taint-remove',
                key: taintKey,
                value: taintValue,
                effect: taintEffect,
              })
            }
          >
            Remove taint
          </Button>
        </div>
        {(action?.kind === 'taint-add' || action?.kind === 'taint-remove') &&
          action.status === 'failed' && (
            <p data-testid="node-action-error" className="text-xs text-status-err">
              {action.error}
            </p>
          )}
      </section>

      <ConfirmDialog
        open={confirm !== undefined}
        title={confirmTitle(confirm)}
        items={[{ namespace: null, name: nodeName }]}
        detail={confirmDetail(confirm)}
        confirmLabel={confirmLabel(confirm)}
        onConfirm={onConfirm}
        onCancel={() => setConfirm(undefined)}
      />
    </div>
  )
}

function confirmTitle(c: PendingConfirm | undefined): string {
  switch (c?.kind) {
    case 'cordon':
      return 'Cordon node'
    case 'uncordon':
      return 'Uncordon node'
    case 'drain':
      return 'Drain node'
    case 'taint-add':
      return 'Add taint'
    case 'taint-remove':
      return 'Remove taint'
    default:
      return ''
  }
}

function confirmLabel(c: PendingConfirm | undefined): string {
  switch (c?.kind) {
    case 'cordon':
      return 'Cordon'
    case 'uncordon':
      return 'Uncordon'
    case 'drain':
      return 'Drain'
    case 'taint-add':
      return 'Add taint'
    case 'taint-remove':
      return 'Remove taint'
    default:
      return 'Confirm'
  }
}

function confirmDetail(c: PendingConfirm | undefined): string | undefined {
  if (!c) return undefined
  if (c.kind === 'drain') {
    return `Evicts every pod on this node${c.ignoreDaemonsets ? ' (DaemonSet pods are skipped)' : ''}. A pod with no owning controller will cause the drain to be refused up front.`
  }
  if (c.kind === 'taint-add') return `${c.key}=${c.value || ''}:${c.effect}`
  if (c.kind === 'taint-remove') return `${c.key}:${c.effect}`
  return undefined
}

function DrainProgressView({
  drain,
  status,
}: {
  drain: NonNullable<NodeAction['drain']>
  status: NodeAction['status']
}) {
  return (
    <div data-testid="drain-progress" className="flex flex-col gap-1 text-xs">
      <p>
        Evicted {String(drain.evicted.length)}/{String(drain.totalPods)}
        {status === 'complete' ? ' — done' : ''}
        {status === 'cancelled' ? ' — cancelled' : ''}
      </p>
      {drain.blockedOn.length > 0 && (
        <div data-testid="drain-blocked" className="flex flex-col gap-0.5">
          <p className="text-status-warn">Blocked:</p>
          {drain.blockedOn.map((b) => (
            <p key={b.pod} className="pl-2 font-mono text-muted-foreground">
              {b.pod}: {b.reason}
            </p>
          ))}
        </div>
      )}
      {drain.ignoredDaemonSetPods.length > 0 && (
        <p data-testid="drain-ignored-daemonsets" className="text-muted-foreground">
          Ignored (DaemonSet): {drain.ignoredDaemonSetPods.join(', ')}
        </p>
      )}
    </div>
  )
}
