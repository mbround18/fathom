/**
 * Fathom's terminals on `@companion/terminal`'s `TerminalSurface`: the pod Terminal tab, the Node
 * Shell panel (037-node-shell T002), and 048's container session. Each caller owns its own
 * banners and controls; the surface reports the session's phase via `onPhase`.
 *
 * A [`TerminalTransport`] here is one kind of session as Fathom's store sees it: how it is
 * started, typed into, resized, stopped, and found again among the store's streams. This module
 * adapts that to the package's store-free surface, which holds the transport in a ref and keys
 * its effects on `key` alone, so inline closures are fine.
 *
 * The backend exec session has no `stream_id` echoed back to the caller, so — exactly like
 * `PodLogsTab` — a transport tracks "its" session by remembering the highest stream id already
 * present when it starts, and watching for a newer entry of its own kind.
 */
import { useRef } from 'react'

import {
  encodeInput,
  TerminalSurface as Surface,
  type TerminalPhase,
  type TerminalStream,
  type TerminalTransport as SurfaceTransport,
} from '@companion/terminal'
// xterm's stylesheet, here rather than in main.tsx so it stays in the terminal's lazy chunk.
import '@companion/terminal/terminal.css'
import { useStore, type ExecStreamState, type StoreState } from '@/store'

export { encodeInput, type TerminalPhase, type TerminalStream }

/**
 * One kind of session, as the terminal needs to see it.
 *
 * `key` is the identity: a change restarts the session. Everything else may be a fresh closure on
 * every render.
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

export interface TerminalSurfaceProps {
  transport: TerminalTransport
  testId?: string
  onPhase?: (phase: TerminalPhase, reason?: string) => void
}

/** A store-backed transport on the package surface. */
export function TerminalSurface({
  transport,
  testId = 'pod-terminal-xterm',
  onPhase,
}: TerminalSurfaceProps) {
  // The stream id "since" which a freshly appearing entry belongs to this session, taken just
  // before the session starts.
  const sinceIdRef = useRef(0)
  const surface: SurfaceTransport = {
    key: transport.key,
    start: () => {
      sinceIdRef.current = maxStreamId(useStore.getState().streams)
      transport.start()
    },
    stop: transport.stop,
    input: transport.input,
    current: () => transport.select(useStore.getState().streams, sinceIdRef.current),
  }
  return (
    <Surface
      transport={surface}
      subscribe={useStore.subscribe}
      onResize={transport.resize}
      testId={testId}
      onPhase={onPhase}
    />
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
