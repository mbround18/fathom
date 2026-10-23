/**
 * Shared xterm.js terminal over one interactive session: the pod Terminal tab, the Node Shell
 * panel (037-node-shell T002), and 048's container session. It renders only the terminal surface
 * and reports the session's phase/reason via `onPhase`; each caller owns its own banners and
 * controls.
 *
 * `TerminalSurface` is the terminal; a [`TerminalTransport`] is how one particular kind of session
 * is started, typed into, resized, stopped and found again in the store. The split exists because
 * 048 needed the same terminal over `docker exec` rather than the Kubernetes API, and everything
 * worth having here — the screen-reader DOM the specs assert against, the fit-on-resize, the
 * "write only what has not been written" drain, the first fit once the session is actually live —
 * is identical in both cases. `TerminalView` is the pod transport wired to it, unchanged.
 *
 * The backend exec session has no `stream_id` echoed back to the caller, so — exactly like
 * `PodLogsTab` — a transport tracks "its" session by remembering the highest stream id already
 * present when it starts, and watching for a newer entry of its own kind.
 */
import { FitAddon } from '@xterm/addon-fit'
import { useEffect, useRef } from 'react'
import { Terminal } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'

import { useStore, type ExecStreamState, type StoreState } from '@/store'
import type { RingBuffer } from '@/store/ring-buffer'

export type TerminalPhase = 'attaching' | 'active' | 'ended' | 'error'

/** What the surface needs of a session, whatever kind it is. */
export interface TerminalStream {
  status: TerminalPhase
  reason?: string
  /** A short-lived handoff queue; xterm.js owns the rendered scrollback (research.md R5). */
  chunks: RingBuffer<string>
  /** Bumped on every appended chunk. */
  rev: number
}

/**
 * One kind of session, as the terminal needs to see it.
 *
 * `key` is the identity: a change restarts the session. Everything else may be a fresh closure on
 * every render — the surface holds the transport in a ref and keys its effects on `key` alone, so
 * an inline callback cannot make the shell reattach on each paint.
 */
export interface TerminalTransport {
  key: string
  start: () => void
  stop: () => void
  input: (data: string) => void
  resize: (cols: number, rows: number) => void
  /** This session's stream, among the entries newer than `sinceId`. */
  select: (streams: StoreState['streams'], sinceId: number) => TerminalStream | undefined
}

export function maxStreamId(streams: Record<number, unknown>): number {
  let max = 0
  for (const key of Object.keys(streams)) max = Math.max(max, Number(key))
  return max
}

/** Base64-encode a keystroke/paste string for an exec session's input (research.md R4). */
export function encodeInput(data: string): string {
  const bytes = new TextEncoder().encode(data)
  let binary = ''
  for (const b of bytes) binary += String.fromCharCode(b)
  return btoa(binary)
}

export interface TerminalSurfaceProps {
  transport: TerminalTransport
  testId?: string
  onPhase?: (phase: TerminalPhase, reason?: string) => void
}

export function TerminalSurface({
  transport,
  testId = 'pod-terminal-xterm',
  onPhase,
}: TerminalSurfaceProps) {
  const containerEl = useRef<HTMLDivElement>(null)
  const terminalRef = useRef<Terminal | null>(null)
  const fitRef = useRef<FitAddon | null>(null)
  // The transport, always current, never a dependency. See `TerminalTransport.key`.
  //
  // Refreshed in an effect with no dependency list rather than assigned during render: an
  // assignment during render is a write the renderer may discard or repeat. This effect is
  // declared first, so by the time the effects below run on a re-render the ref already holds
  // this render's transport, and `useRef`'s initial value covers the first mount.
  const transportRef = useRef(transport)
  useEffect(() => {
    transportRef.current = transport
  })

  // Create the xterm.js instance once, for the component's whole lifetime.
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

    const dataDisposable = term.onData((data) => transportRef.current.input(data))
    // FR-010: propagate every size change to the remote shell (a no-op backend-side if no session
    // is active yet, per contracts/protocol.md's "implicit target" commands).
    const resizeDisposable = term.onResize(({ cols, rows }) =>
      transportRef.current.resize(cols, rows),
    )

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
  }, [])

  // The stream id "since" which a freshly appearing entry belongs to this session.
  const sinceIdRef = useRef(0)
  const consumedRef = useRef(0)

  useEffect(() => {
    sinceIdRef.current = maxStreamId(useStore.getState().streams)
    consumedRef.current = 0
    terminalRef.current?.clear()
    const transport = transportRef.current
    transport.start()
    return () => transport.stop()
  }, [transport.key])

  const stream = useStore((s) => transportRef.current.select(s.streams, sinceIdRef.current))

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
    transportRef.current.resize(term.cols, term.rows)
  }, [phase])

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

export interface TerminalViewProps {
  namespace: string
  name: string
  /** Container to attach to; `null` lets the backend pick the pod's only container. */
  container: string | null
  testId?: string
  onPhase?: (phase: TerminalPhase, reason?: string) => void
}

/** The pod exec transport on the shared surface. */
export function TerminalView({
  namespace,
  name,
  container,
  testId = 'pod-terminal-xterm',
  onPhase,
}: TerminalViewProps) {
  const send = useStore((s) => s.send)
  const transport: TerminalTransport = {
    key: `${namespace}/${name}/${container ?? ''}`,
    start: () => send({ type: 'StartPodExec', data: { namespace, name, container } }),
    stop: () => send({ type: 'StopPodExec' }),
    input: (data) => send({ type: 'SendPodExecInput', data: { data: encodeInput(data) } }),
    resize: (cols, rows) => send({ type: 'ResizePodExec', data: { cols, rows } }),
    select: (streams, sinceId) => {
      let bestId = -1
      let best: ExecStreamState | undefined
      for (const key of Object.keys(streams)) {
        const id = Number(key)
        if (id <= sinceId) continue
        const entry = streams[id]
        if (entry?.kind !== 'exec') continue
        if (id > bestId) {
          bestId = id
          best = entry
        }
      }
      return best
    },
  }
  return <TerminalSurface transport={transport} testId={testId} onPhase={onPhase} />
}
