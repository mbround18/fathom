/**
 * Terminal tab (Phase 4, US2): an interactive `kube exec` shell rendered with xterm.js,
 * driven by `StartPodExec`/`SendPodExecInput`/`ResizePodExec`/`StopPodExec` and the
 * `PodExecReady`/`PodExecOutputChunk`/`StreamClosed` events (contracts/protocol.md).
 *
 * The terminal surface and stream tracking live in the shared `TerminalView`
 * (037-node-shell T002); this tab adds the container picker, the 024 pre-session gate
 * (permission pre-check -> confirmation), the 033 capability gate and the ended/error banner.
 */
import { PlugsIcon, ShieldWarningIcon, WarningIcon } from '@phosphor-icons/react'
import { useEffect, useState } from 'react'

import { Button } from '@companion/ui'
import { useCapability } from '@/lib/capabilities'
import type { PodDetails } from '@/protocol/types'
import type { ObjectRef } from '@/resources'
import { detailKey, execAccessKey, useStore } from '@/store'

import { ConfirmDialog } from '../ConfirmDialog'
import { Picker } from '../Picker'
import { Loading } from '../shell/PaneStates'
import { TerminalView, type TerminalPhase } from './TerminalView'

/** 024-pod-exec-terminal: pre-session gate; `open` means confirmed, session may start. */
type Gate = 'idle' | 'checking' | 'denied' | 'confirming' | 'open'
type Phase = Gate | 'picking' | TerminalPhase

/** FR-006 fail-safe: a pre-check that never answers is treated as denied. */
const CHECK_TIMEOUT_MS = 15000
const CHECK_FAILED_REASON = 'Could not verify permission to open a shell in this container.'

export function PodTerminalTab({ target }: { target: ObjectRef }) {
  const namespace = target.namespace ?? ''
  const name = target.name
  const send = useStore((s) => s.send)
  const context = useStore((s) => s.session.activeContext)
  const detailState = useStore((s) => s.details[detailKey(context, 'Pod', namespace, name)])
  const detail =
    detailState?.eventType === 'PodDetailsUpdated' ? (detailState.data as PodDetails) : undefined
  const containers = detail?.containers ?? []

  // FR-008: with more than one container, the operator picks one before the session starts.
  const [picked, setPicked] = useState<string | undefined>(undefined)
  const chosen = containers.length > 1 ? picked : containers[0]
  const started = chosen !== undefined

  // 033 US5: the session only starts once the identity is verified to `create pods/exec`.
  const canExec = useCapability({
    verb: 'create',
    resource: 'pods',
    subresource: 'exec',
    namespace,
  })

  // 024-pod-exec-terminal gate: check access -> confirm -> only then StartPodExec.
  const container = containers.length > 1 ? (chosen ?? null) : null
  const accessKey = execAccessKey(namespace, name, container)
  const access = useStore((s) => s.execAccess[accessKey])
  const [attempt, setAttempt] = useState(0)
  const runKey = started ? `${accessKey}#${attempt}` : ''
  const [run, setRun] = useState({
    key: '',
    baseline: 0,
    idle: false,
    confirmed: false,
    timedOut: false,
  })
  // Every (re)open, or pod/container change, starts a fresh run with a fresh pre-check; a
  // stale confirming/denied result is never reused (confirmation is never skipped). Adjusting
  // state during render (rather than in an effect) is React's documented reset pattern.
  if (run.key !== runKey) {
    setRun({
      key: runKey,
      baseline: access?.rev ?? 0,
      idle: false,
      confirmed: false,
      timedOut: false,
    })
  }
  const fresh = access !== undefined && access.rev > run.baseline

  const gate: Gate =
    !started || run.idle
      ? 'idle'
      : run.confirmed
        ? 'open'
        : fresh
          ? access.allowed
            ? 'confirming'
            : 'denied'
          : run.timedOut
            ? 'denied'
            : 'checking'
  const deniedReason = fresh ? access.reason : CHECK_FAILED_REASON

  const beginCheck = () => setAttempt((n) => n + 1)

  useEffect(() => {
    if (!runKey) return
    send({ type: 'CheckPodExecAccess', data: { namespace, name, container } })
  }, [runKey, namespace, name, container, send])

  useEffect(() => {
    if (gate !== 'checking') return
    const t = setTimeout(() => setRun((r) => ({ ...r, timedOut: true })), CHECK_TIMEOUT_MS)
    return () => clearTimeout(t)
  }, [gate, runKey])

  const [status, setStatus] = useState<{ phase: TerminalPhase; reason?: string }>({
    phase: 'attaching',
  })
  const open = gate === 'open' && canExec.status === 'allowed'
  const phase: Phase = !started
    ? 'picking'
    : gate !== 'open'
      ? gate
      : open
        ? status.phase
        : 'checking'

  const cancelConfirm = () => {
    if (containers.length > 1) setPicked(undefined)
    else setRun((r) => ({ ...r, idle: true }))
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {canExec.status !== 'allowed' && (
        <p
          data-testid="terminal-capability-gate"
          data-capability={canExec.status}
          className="shrink-0 border-b border-border px-3 py-2 text-xs text-muted-foreground"
        >
          {canExec.reason}
        </p>
      )}
      {containers.length > 1 && !started && (
        <div className="flex shrink-0 items-center gap-2 border-b border-border px-2 py-1.5">
          <Picker
            label="Container"
            prefix="container"
            value={picked}
            options={containers.map((c) => ({ value: c, label: c }))}
            onChange={setPicked}
          />
          <span className="text-xs text-muted-foreground">
            Choose a container to attach the terminal to.
          </span>
        </div>
      )}
      <div className="relative min-h-0 flex-1 bg-background/40">
        {phase === 'picking' && containers.length === 0 && <Loading rows={6} label="Loading pod" />}
        {open && (
          <TerminalView
            key={`${namespace}/${name}/${chosen}`}
            namespace={namespace}
            name={name}
            container={container}
            onPhase={(p, reason) => setStatus({ phase: p, reason })}
          />
        )}
        {phase === 'idle' && (
          <div className="absolute inset-0 flex items-center justify-center">
            <Button type="button" variant="outline" size="sm" onClick={beginCheck}>
              Open Terminal
            </Button>
          </div>
        )}
        {phase === 'checking' && (
          <div className="absolute inset-0 flex items-center justify-center bg-background/60 text-xs text-muted-foreground">
            Checking permissions…
          </div>
        )}
        {phase === 'denied' && (
          <div
            role="alert"
            data-pane-state="terminal-denied"
            className="absolute inset-0 flex flex-col items-center justify-center gap-2 px-4 text-center text-xs text-status-warn"
          >
            <ShieldWarningIcon aria-hidden="true" className="size-5" />
            <span>
              {deniedReason ?? "You don't have permission to open a shell in this container."}
            </span>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => (containers.length > 1 ? setPicked(undefined) : beginCheck())}
            >
              {containers.length > 1 ? 'Choose container' : 'Retry'}
            </Button>
          </div>
        )}
        {open && phase === 'attaching' && (
          <div className="absolute inset-0 flex items-center justify-center bg-background/60 text-xs text-muted-foreground">
            Attaching shell…
          </div>
        )}
      </div>
      {(phase === 'ended' || phase === 'error') && (
        <div
          role="status"
          data-pane-state={phase === 'error' ? 'terminal-error' : 'terminal-ended'}
          className="flex shrink-0 items-center gap-2 border-t border-status-warn/40 bg-status-warn/10 px-3 py-1.5 text-xs text-status-warn"
        >
          {phase === 'error' ? (
            status.reason?.toLowerCase().includes('permission') ? (
              <ShieldWarningIcon aria-hidden="true" className="size-4" />
            ) : (
              <WarningIcon aria-hidden="true" className="size-4" />
            )
          ) : (
            <PlugsIcon aria-hidden="true" className="size-4" />
          )}
          {/* FR-013: the backend's error reason is shown verbatim, not a generic message. */}
          <span>
            {phase === 'error' ? (status.reason ?? 'Failed to attach.') : 'Session ended.'}
          </span>
          {containers.length > 1 && (
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="ml-auto"
              onClick={() => setPicked(undefined)}
            >
              Choose container
            </Button>
          )}
        </div>
      )}
      <ConfirmDialog
        open={phase === 'confirming'}
        title="Open a shell in this container?"
        items={[{ namespace, name }]}
        detail={
          <>
            This opens an interactive shell in container{' '}
            <strong>{chosen ?? containers[0] ?? 'default'}</strong>. Anything you type runs inside
            the pod.
          </>
        }
        destructive
        confirmLabel="Open terminal"
        onConfirm={() => setRun((r) => ({ ...r, confirmed: true }))}
        onCancel={cancelConfirm}
      />
    </div>
  )
}
