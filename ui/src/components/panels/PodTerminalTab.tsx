/**
 * Terminal tab (Phase 4, US2): an interactive `kube exec` shell rendered with xterm.js,
 * driven by `StartPodExec`/`SendPodExecInput`/`ResizePodExec`/`StopPodExec` and the
 * `PodExecReady`/`PodExecOutputChunk`/`StreamClosed` events (contracts/protocol.md).
 *
 * The backend exec session is a singleton (state.exec_session, `src/backend/exec.rs`) with
 * no `stream_id` echoed back to the caller, so — exactly like `PodLogsTab` — this component
 * tracks "its" session by remembering the highest stream id already present when it sends
 * `StartPodExec` and watching for a newer `kind: 'exec'` entry to appear.
 */
import { PlugsIcon, ShieldWarningIcon, WarningIcon } from '@phosphor-icons/react'
import { FitAddon } from '@xterm/addon-fit'
import { useEffect, useMemo, useRef, useState } from 'react'
import { Terminal } from 'xterm'
import 'xterm/css/xterm.css'

import { Button } from '@/components/ui/button'
import type { PodDetails } from '@/protocol/types'
import type { ObjectRef } from '@/resources'
import { detailKey, execAccessKey, useStore, type ExecStreamState } from '@/store'

import { ConfirmDialog } from '../ConfirmDialog'
import { Picker } from '../Picker'
import { Loading } from '../shell/PaneStates'

function maxStreamId(streams: Record<number, unknown>): number {
  let max = 0
  for (const key of Object.keys(streams)) max = Math.max(max, Number(key))
  return max
}

/** Base64-encode a keystroke/paste string for `SendPodExecInput.data` (research.md R4). */
function encodeInput(data: string): string {
  const bytes = new TextEncoder().encode(data)
  let binary = ''
  for (const b of bytes) binary += String.fromCharCode(b)
  return btoa(binary)
}

type Phase =
  | 'picking'
  | 'idle'
  | 'checking'
  | 'denied'
  | 'confirming'
  | 'attaching'
  | 'active'
  | 'ended'
  | 'error'

/** 024-pod-exec-terminal: pre-session gate; `open` means confirmed, session may start. */
type Gate = 'idle' | 'checking' | 'denied' | 'confirming' | 'open'

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

  const containerEl = useRef<HTMLDivElement>(null)
  const terminalRef = useRef<Terminal | null>(null)
  const fitRef = useRef<FitAddon | null>(null)

  // Create the xterm.js instance once, for the component's whole lifetime. `send` is a
  // stable function reference from the store, so this effect only ever runs once/unmount.
  useEffect(() => {
    const el = containerEl.current
    if (!el) return
    // `screenReaderMode` also gives the terminal an accessible text DOM tree (on top of the
    // canvas rendering), which Playwright specs assert against instead of a screenshot.
    const term = new Terminal({
      scrollback: 1000,
      cursorBlink: true,
      convertEol: true,
      screenReaderMode: true,
    })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(el)
    fit.fit()
    terminalRef.current = term
    fitRef.current = fit

    const dataDisposable = term.onData((data) => {
      send({ type: 'SendPodExecInput', data: { data: encodeInput(data) } })
    })
    // FR-010: propagate every size change to the remote shell (no-op backend-side if no
    // session is active yet, per contracts/protocol.md's "implicit target" commands).
    const resizeDisposable = term.onResize(({ cols, rows }) => {
      send({ type: 'ResizePodExec', data: { cols, rows } })
    })

    const ro = new ResizeObserver(() => fitRef.current?.fit())
    ro.observe(el)

    return () => {
      ro.disconnect()
      dataDisposable.dispose()
      resizeDisposable.dispose()
      term.dispose()
      terminalRef.current = null
      fitRef.current = null
    }
  }, [send])

  // The stream id "since" which a freshly appearing 'exec' entry belongs to this session.
  const sinceIdRef = useRef(0)
  const consumedRef = useRef(0)

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
    setRun({ key: runKey, baseline: access?.rev ?? 0, idle: false, confirmed: false, timedOut: false })
  }
  const fresh = access !== undefined && access.rev > run.baseline

  const gate: Gate = !started || run.idle
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

  useEffect(() => {
    if (gate !== 'open') return
    sinceIdRef.current = maxStreamId(useStore.getState().streams)
    consumedRef.current = 0
    terminalRef.current?.clear()
    send({ type: 'StartPodExec', data: { namespace, name, container } })
    return () => send({ type: 'StopPodExec' })
  }, [gate, namespace, name, container, send])

  const stream = useStore((s) => {
    let bestId = -1
    let best: ExecStreamState | undefined
    for (const key of Object.keys(s.streams)) {
      const id = Number(key)
      if (id <= sinceIdRef.current) continue
      const entry = s.streams[id]
      if (entry?.kind !== 'exec') continue
      if (id > bestId) {
        bestId = id
        best = entry
      }
    }
    return best
  })

  // Write newly-arrived output chunks to the terminal (data-model.md: the store only holds
  // a short-lived handoff queue, xterm.js itself owns the rendered scrollback).
  useEffect(() => {
    const term = terminalRef.current
    if (!term || !stream) return
    const buf = stream.chunks
    const total = buf.truncated + buf.length
    const start = Math.max(consumedRef.current, buf.truncated)
    for (let j = start; j < total; j++) {
      const item = buf.at(j - buf.truncated)
      if (item !== undefined) term.write(item)
    }
    consumedRef.current = total
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on rev, reads buf by ref
  }, [stream?.rev])

  const phase: Phase = useMemo(() => {
    if (!started) return 'picking'
    if (gate !== 'open') return gate
    if (!stream) return 'attaching'
    return stream.status
  }, [started, gate, stream])

  const cancelConfirm = () => {
    if (containers.length > 1) setPicked(undefined)
    else setRun((r) => ({ ...r, idle: true }))
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
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
        <div
          ref={containerEl}
          data-testid="pod-terminal-xterm"
          className="h-full min-w-0 p-2"
          hidden={phase === 'picking' || phase === 'idle' || phase === 'denied'}
        />
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
        {phase === 'attaching' && (
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
            stream?.reason?.toLowerCase().includes('permission') ? (
              <ShieldWarningIcon aria-hidden="true" className="size-4" />
            ) : (
              <WarningIcon aria-hidden="true" className="size-4" />
            )
          ) : (
            <PlugsIcon aria-hidden="true" className="size-4" />
          )}
          {/* FR-013: the backend's error reason is shown verbatim, not a generic message. */}
          <span>
            {phase === 'error' ? (stream?.reason ?? 'Failed to attach.') : 'Session ended.'}
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
