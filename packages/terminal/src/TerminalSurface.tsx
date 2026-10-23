/**
 * An xterm.js terminal over one interactive session, free of any app store.
 *
 * The caller brings a [`TerminalTransport`] (how this session is started, typed into, stopped and
 * read), a `subscribe` that says when the session's stream may have changed, and an `onResize`
 * that carries the terminal's size to the remote pty. The surface renders the terminal, writes
 * only the output it has not yet written, fits itself to its box, and reports the session's phase.
 * Banners and controls stay with the caller.
 *
 * Import `@companion/terminal/terminal.css` once from the app's entry for xterm's own styles; this
 * module imports no CSS so that the package stays importable outside a bundler.
 */
// Namespace imports: xterm ships CommonJS to Node, whose named exports Node cannot detect. A
// namespace import keeps this module (and the package index) importable outside a bundler, e.g.
// from an end-to-end spec; the classes are only touched in effects, in a browser.
import * as fitAddon from '@xterm/addon-fit'
import * as xterm from '@xterm/xterm'
import { useEffect, useRef, useSyncExternalStore } from 'react'

import type { RingBuffer } from './ring-buffer'

export type TerminalPhase = 'attaching' | 'active' | 'ended' | 'error'

/** What the surface needs of a session, whatever kind it is. */
export interface TerminalStream {
  status: TerminalPhase
  reason?: string
  /** A short-lived handoff queue; xterm.js owns the rendered scrollback. */
  chunks: RingBuffer<string>
  /** Bumped on every appended chunk. */
  rev: number
}

/**
 * One session, as the terminal sees it.
 *
 * `key` is the identity: a change restarts the session. Everything else may be a fresh closure on
 * every render; the surface holds the transport in a ref and keys its effects on `key` alone, so
 * an inline callback cannot make the shell reattach on each paint.
 */
export interface TerminalTransport {
  key: string
  start: () => void
  stop: () => void
  input: (data: string) => void
  /** The session's stream as it is now; `undefined` until it exists. */
  current: () => TerminalStream | undefined
}

export interface TerminalSurfaceProps {
  transport: TerminalTransport
  /** Calls `onChange` whenever `transport.current()` may return something new; returns an unsubscribe. */
  subscribe: (onChange: () => void) => () => void
  /** The terminal's size changed, or the session became live at this size. */
  onResize: (cols: number, rows: number) => void
  testId?: string
  onPhase?: (phase: TerminalPhase, reason?: string) => void
}

/** Base64-encode a keystroke/paste string for an exec session's input. */
export function encodeInput(data: string): string {
  const bytes = new TextEncoder().encode(data)
  let binary = ''
  for (const b of bytes) binary += String.fromCharCode(b)
  return btoa(binary)
}

export function TerminalSurface({
  transport,
  subscribe,
  onResize,
  testId = 'terminal-xterm',
  onPhase,
}: TerminalSurfaceProps) {
  const containerEl = useRef<HTMLDivElement>(null)
  const terminalRef = useRef<xterm.Terminal | null>(null)
  const fitRef = useRef<fitAddon.FitAddon | null>(null)
  // The transport and resize callback, always current, never a dependency. Refreshed in an effect
  // with no dependency list rather than assigned during render (a render may be discarded or
  // repeated). Declared first, so the effects below see this render's values.
  const transportRef = useRef(transport)
  const onResizeRef = useRef(onResize)
  useEffect(() => {
    transportRef.current = transport
    onResizeRef.current = onResize
  })

  // Create the xterm.js instance once, for the component's whole lifetime.
  useEffect(() => {
    const el = containerEl.current
    if (!el) return
    // `screenReaderMode` also gives the terminal an accessible text DOM tree (on top of the
    // canvas rendering), which end-to-end specs assert against instead of a screenshot.
    const term = new xterm.Terminal({
      scrollback: 1000,
      cursorBlink: true,
      convertEol: true,
      screenReaderMode: true,
    })
    const fit = new fitAddon.FitAddon()
    term.loadAddon(fit)
    term.open(el)
    fit.fit()
    terminalRef.current = term
    fitRef.current = fit

    const dataDisposable = term.onData((data) => transportRef.current.input(data))
    // Propagate every size change to the remote shell.
    const resizeDisposable = term.onResize(({ cols, rows }) => onResizeRef.current(cols, rows))

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

  const consumedRef = useRef(0)

  useEffect(() => {
    consumedRef.current = 0
    terminalRef.current?.clear()
    const transport = transportRef.current
    transport.start()
    return () => transport.stop()
  }, [transport.key])

  const stream = useSyncExternalStore(subscribe, () => transportRef.current.current())

  // Write newly arrived output chunks to the terminal; the stream only holds a short-lived
  // handoff queue, xterm.js itself owns the rendered scrollback.
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

  // The first fit ran before the session existed, so its resize went nowhere and the remote pty
  // kept its default size. Send the current size once the session is live.
  useEffect(() => {
    const term = terminalRef.current
    if (phase !== 'active' || !term) return
    fitRef.current?.fit()
    onResizeRef.current(term.cols, term.rows)
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
