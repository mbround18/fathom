/**
 * The demo chat's terminal view (015-mcp-agent-exposure, US4/T051).
 *
 * Structurally `PodTerminalTab`'s xterm.js half, with the cluster removed: the process on the
 * other end is a local `claude` subprocess started by `StartMcpDemoChat`, not a `kube exec`
 * session, so it is driven by the `McpDemoChat*` events instead of `PodExec*`/`StreamClosed`.
 *
 * Two things are reused verbatim because they are the load-bearing parts of that pattern:
 *
 * * **xterm.js owns the scrollback.** The store keeps only a bounded handoff queue
 *   (`mcp.demoChat.chunks`) that this component drains into the terminal as chunks arrive,
 *   tracking how far it has consumed so a re-render never rewrites output.
 * * **Sessions are told apart by a monotonic id.** `mcp.demoChat.sessionId` replaces
 *   `PodTerminalTab`'s "highest stream id I started" bookkeeping — the backend hands the id out
 *   directly here, so a new session clears the terminal and the consumed cursor together, and a
 *   previous run's trailing chunks cannot bleed in.
 *
 * Unlike the pod terminal, the subprocess is attached through pipes rather than a pty, so
 * `ResizeMcpDemoChat` records the size for the *next* launch (see `demo_chat.rs::resize`)
 * rather than resizing a live remote tty.
 */
import { FitAddon } from '@xterm/addon-fit'
import { useEffect, useRef } from 'react'
import { Terminal } from '@xterm/xterm'
import '@xterm/xterm/css/xterm.css'

import { useStore } from '@/store'

/** Base64-encode a keystroke/paste for `SendMcpDemoChatInput.data`, as `PodTerminalTab` does. */
function encodeInput(data: string): string {
  const bytes = new TextEncoder().encode(data)
  let binary = ''
  for (const b of bytes) binary += String.fromCharCode(b)
  return btoa(binary)
}

export function DemoChatTerminal() {
  const send = useStore((s) => s.send)
  const demoChat = useStore((s) => s.mcp.demoChat)

  const containerEl = useRef<HTMLDivElement>(null)
  const terminalRef = useRef<Terminal | null>(null)
  const fitRef = useRef<FitAddon | null>(null)

  // One xterm.js instance for the component's whole lifetime; `send` is a stable store
  // reference, so this runs once per mount.
  useEffect(() => {
    const el = containerEl.current
    if (!el) return
    // `screenReaderMode` gives the terminal an accessible text DOM tree alongside the canvas,
    // which the Playwright specs assert against instead of a screenshot.
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
      send({ type: 'SendMcpDemoChatInput', data: { data: encodeInput(data) } })
    })
    const resizeDisposable = term.onResize(({ cols, rows }) => {
      send({ type: 'ResizeMcpDemoChat', data: { cols, rows } })
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

  // How much of the handoff queue has already been written, and for which session.
  const consumedRef = useRef(0)
  const sessionRef = useRef<number | undefined>(undefined)

  useEffect(() => {
    const term = terminalRef.current
    if (!term) return
    if (sessionRef.current !== demoChat.sessionId) {
      sessionRef.current = demoChat.sessionId
      consumedRef.current = 0
      term.clear()
    }
    const buf = demoChat.chunks
    const total = buf.truncated + buf.length
    const start = Math.max(consumedRef.current, buf.truncated)
    for (let j = start; j < total; j++) {
      const item = buf.at(j - buf.truncated)
      if (item !== undefined) term.write(item)
    }
    consumedRef.current = total
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on rev, reads the buffer by ref
  }, [demoChat.rev, demoChat.sessionId])

  return (
    <div className="flex min-h-0 flex-col gap-1" data-testid="mcp-demo-chat-terminal">
      <div
        ref={containerEl}
        data-testid="mcp-demo-chat-xterm"
        className="h-56 min-w-0 rounded border border-border bg-background/40 p-2"
      />
      {demoChat.status === 'ended' && (
        <p role="status" data-testid="mcp-demo-chat-ended" className="text-xs text-status-warn">
          {/* The backend's reason verbatim — including a usage error from the real `claude`
              CLI, which is the honest failure mode for a renamed flag. */}
          Example chat ended: {demoChat.reason ?? 'exit'}
        </p>
      )}
    </div>
  )
}
