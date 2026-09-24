/**
 * "Available Agents" section of Cluster Settings (015-mcp-agent-exposure, US1/T027).
 *
 * Answers the operator's question "which agents can reach this cluster, and how?" in one place:
 *
 * * **The stdio MCP server** — off by default (FR-002). The toggle is the *only* thing in the app
 *   that can start it: flipping it sends `SetMcpServerEnabled`, and the backend persists the
 *   preference and starts/stops the server task in the same command, then broadcasts
 *   `McpServerStatusChanged`. `running` is rendered from that broadcast, never inferred from the
 *   toggle, so a start that failed shows as "enabled, not running" rather than a false green.
 * * **The connection command** — the literal `fathom --only-mcp` invocation an external MCP
 *   client (Claude Code, Claude Desktop, …) should launch, copyable as a snippet (FR-007). Shown
 *   only while the server is enabled: a snippet for a server that is off would be a trap.
 * * **The in-app assistant's auth** — a read-only summary of `agentAuth.mode`, the same slice
 *   `agent-ops/AuthSetup.tsx` owns. This section never writes it; changing it stays in AuthSetup
 *   (one owner per setting), and the endpoint/model are shown while the API key never is
 *   (Principle VI — only AuthSetup's explicit "Reveal key" click can surface that).
 *
 * All testids are prefixed `mcp-`.
 */
import { useEffect, useState } from 'react'

import { Button } from '@/components/ui/button'
import { useStore } from '@/store'
import { waitForCommandResult } from '@/store/actions'

import { DemoChatTerminal } from './DemoChatTerminal'

/** One line describing how the in-app assistant is configured, without ever naming a secret. */
function authSummary(mode: { type: string; data?: unknown } | undefined): string {
  switch (mode?.type) {
    case 'Anthropic':
      return 'Anthropic (Claude) — using your own API key'
    case 'Ollama': {
      const data = mode.data as { endpoint: string; model: string }
      return `Ollama — ${data.model} at ${data.endpoint}`
    }
    case 'Disabled':
      return 'Disabled — the in-app assistant will not answer'
    default:
      return 'Not configured yet'
  }
}

export function AvailableAgentsSection() {
  const request = useStore((s) => s.request)
  const send = useStore((s) => s.send)
  const mcp = useStore((s) => s.mcp)
  const authMode = useStore((s) => s.agentAuth.mode)
  const persistedEnabled = useStore((s) => s.prefs.mcpStdioEnabled)

  const [toggling, setToggling] = useState(false)
  const [error, setError] = useState<string | undefined>(undefined)
  const [copied, setCopied] = useState(false)

  // Read-only on mount. `LoadMcpServerStatus` deliberately cannot start the server — the
  // backend's handler only reads the persisted preference and the live task's presence.
  useEffect(() => {
    request({ type: 'LoadMcpServerStatus' }).catch(() => {
      /* best-effort: the section still renders, showing the off/unknown state */
    })
    // FR-008: reuse whatever the assistant's own setup already loaded. Only an empty slice is
    // worth a read; re-asking would be a duplicate credential lookup for a summary this
    // section cannot even edit.
    if (useStore.getState().agentAuth.mode === undefined) {
      request({ type: 'AgentGetAuthMode' }).catch(() => {
        /* best-effort: the auth summary falls back to "Not configured yet" */
      })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // Until the first `McpServerStatusChanged` lands, show the persisted preference from
  // `UiConfigLoaded` rather than a default-off toggle: after a restart with the server on, the
  // operator must never see it as off and be tempted to re-flip it (US3 acceptance scenario 2).
  const enabled = mcp.loaded ? mcp.enabled : persistedEnabled

  const setEnabled = (enabled: boolean) => {
    if (toggling) return
    setToggling(true)
    setError(undefined)
    void (async () => {
      try {
        const requestId = await request({ type: 'SetMcpServerEnabled', data: { enabled } })
        const result = await waitForCommandResult(requestId)
        if (result === 'timeout') setError('The backend did not respond in time.')
        else if (!result.ok)
          setError(result.error ?? `Could not turn the MCP server ${enabled ? 'on' : 'off'}.`)
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setToggling(false)
      }
    })()
  }

  const demoChat = mcp.demoChat

  const copyCommand = () => {
    void navigator.clipboard
      .writeText(mcp.connectionCommand)
      .then(() => setCopied(true))
      .catch(() => setError('Could not copy to the clipboard.'))
  }

  return (
    <fieldset className="flex flex-col gap-3" data-testid="mcp-available-agents">
      <legend className="text-xs font-semibold tracking-wider uppercase">Available Agents</legend>

      <label className="flex items-center gap-2 text-xs">
        <input
          type="checkbox"
          data-testid="mcp-server-toggle"
          checked={enabled}
          disabled={toggling}
          onChange={(e) => setEnabled(e.target.checked)}
        />
        Expose this cluster to external agents over MCP (stdio)
      </label>

      <p className="text-xs" data-testid="mcp-server-state">
        {enabled
          ? mcp.running
            ? 'Running — attached MCP clients can read this cluster.'
            : 'Enabled, but not running.'
          : 'Off — nothing is listening, and no agent can reach this cluster.'}
      </p>

      {enabled && mcp.running && (
        <span
          data-testid="mcp-running-indicator"
          className="w-fit rounded-full bg-emerald-500/15 px-2 py-0.5 text-[10px] font-semibold tracking-wider text-emerald-400 uppercase"
        >
          Running
        </span>
      )}

      {enabled && mcp.connectionCommand !== '' && (
        <div className="flex flex-col gap-1 text-xs">
          <span>Connect an external MCP client with:</span>
          <code
            data-testid="mcp-connection-command"
            className="overflow-x-auto rounded bg-muted px-2 py-1 font-mono text-[11px]"
          >
            {mcp.connectionCommand}
          </code>
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              data-testid="mcp-copy-connection-command"
              onClick={copyCommand}
            >
              Copy
            </Button>
            {copied && <span data-testid="mcp-copied">Copied.</span>}
          </div>
        </div>
      )}

      <div className="flex flex-col gap-2 text-xs" data-testid="mcp-demo-chat">
        <span>Example chat</span>
        {!mcp.running ? (
          <p data-testid="mcp-demo-chat-server-off" className="text-muted-foreground">
            Turn the MCP server on first — an example chat would have nothing to attach to.
          </p>
        ) : demoChat.status === 'not-detected' ? (
          <p data-testid="mcp-demo-chat-not-detected" className="text-muted-foreground">
            {demoChat.instructions ??
              'No `claude` executable was found on this machine\u2019s PATH, so there is nothing to launch.'}
          </p>
        ) : (
          <div className="flex items-center gap-2">
            <Button
              type="button"
              variant="outline"
              size="sm"
              data-testid="mcp-demo-chat-launch"
              onClick={() => send({ type: 'StartMcpDemoChat' })}
            >
              {demoChat.status === 'running' || demoChat.status === 'ended'
                ? 'Relaunch example chat'
                : 'Launch example chat'}
            </Button>
            {demoChat.status === 'running' && (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                data-testid="mcp-demo-chat-stop"
                onClick={() => send({ type: 'StopMcpDemoChat' })}
              >
                Stop
              </Button>
            )}
          </div>
        )}
        {mcp.running && (demoChat.status === 'running' || demoChat.status === 'ended') && (
          <DemoChatTerminal />
        )}
      </div>

      <div className="flex flex-col gap-1 text-xs">
        <span>In-app assistant</span>
        <span data-testid="mcp-agent-auth-summary" className="text-muted-foreground">
          {authSummary(authMode)}
        </span>
        <span className="text-[11px] text-muted-foreground">
          Change this in the assistant&apos;s own setup, from the Ask AI tab.
        </span>
      </div>

      {error !== undefined && (
        <p data-testid="mcp-error" className="text-xs text-destructive">
          {error}
        </p>
      )}
    </fieldset>
  )
}
