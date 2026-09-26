/**
 * Shared xterm.js terminal over the singleton exec session (037-node-shell T002): used by the
 * pod Terminal tab and the Node Shell panel. It renders only the terminal surface and reports
 * the session's phase/reason via `onPhase`; each caller owns its own banners and controls.
 *
 * The backend exec session has no `stream_id` echoed back to the caller, so — exactly like
 * `PodLogsTab` — this component tracks "its" session by remembering the highest stream id
 * already present when it sends `StartPodExec` and watching for a newer `kind: 'exec'` entry.
 */
import { FitAddon } from '@xterm/addon-fit'
import { useEffect, useRef } from 'react'
import { Terminal } from 'xterm'
import 'xterm/css/xterm.css'

import { useStore, type ExecStreamState } from '@/store'

export type TerminalPhase = 'attaching' | 'active' | 'ended' | 'error'

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

export interface TerminalViewProps {
  namespace: string
  name: string
  /** Container to attach to; `null` lets the backend pick the pod's only container. */
  container: string | null
  testId?: string
  onPhase?: (phase: TerminalPhase, reason?: string) => void
}

export function TerminalView({
  namespace,
  name,
  container,
  testId = 'pod-terminal-xterm',
  onPhase,
}: TerminalViewProps) {
  const send = useStore((s) => s.send)
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

  useEffect(() => {
    sinceIdRef.current = maxStreamId(useStore.getState().streams)
    consumedRef.current = 0
    terminalRef.current?.clear()
    send({ type: 'StartPodExec', data: { namespace, name, container } })
    return () => send({ type: 'StopPodExec' })
  }, [namespace, name, container, send])

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

  const phase: TerminalPhase = stream === undefined ? 'attaching' : stream.status
  const reason = stream?.reason

  // FR-010: the fit above ran before the session existed, so its resize was a backend no-op
  // and the remote pty kept its default 80x24 until the pane next changed size. Send the
  // current size once the session is live.
  useEffect(() => {
    const term = terminalRef.current
    if (phase !== 'active' || !term) return
    fitRef.current?.fit()
    send({ type: 'ResizePodExec', data: { cols: term.cols, rows: term.rows } })
  }, [phase, send])

  useEffect(() => {
    onPhase?.(phase, reason)
    // eslint-disable-next-line react-hooks/exhaustive-deps -- callers pass inline callbacks
  }, [phase, reason])

  return (
    <div className="relative min-h-0 flex-1 bg-background/40">
      <div ref={containerEl} data-testid={testId} className="h-full min-w-0 p-2" />
      {phase === 'attaching' && (
        <div className="absolute inset-0 flex items-center justify-center bg-background/60 text-xs text-muted-foreground">
          Attaching shell…
        </div>
      )}
    </div>
  )
}
