/**
 * `AvailableAgentsSection` (015-mcp-agent-exposure, US1/T030): the Cluster Settings section that
 * exposes the stdio MCP server's toggle, its live running state, the connection snippet and a
 * read-only summary of the in-app assistant's auth.
 *
 * The load-bearing assertions are the "off" ones: on mount the section sends only *read*
 * commands (never `SetMcpServerEnabled`), shows nothing listening, and shows no connection
 * snippet. `SetMcpServerEnabled { enabled: true }` is dispatched by the operator's click and
 * nothing else (FR-002).
 *
 * Mirrors `authSetup.test.tsx`'s conventions: store reset per test, `request` stubbed to record
 * commands, no real transport.
 */
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

// `DemoChatTerminal` mounts a real xterm.js instance, which needs a canvas and `matchMedia`
// that jsdom does not provide. These tests are about the *section's* states, not xterm's
// rendering (the e2e specs exercise the real terminal in a browser), so the terminal's own
// dependencies are stubbed and the component under test is otherwise untouched.
vi.mock('xterm', () => ({
  Terminal: class {
    open() {}
    clear() {}
    write() {}
    dispose() {}
    loadAddon() {}
    onData() {
      return { dispose() {} }
    }
    onResize() {
      return { dispose() {} }
    }
  },
}))
vi.mock('xterm/css/xterm.css', () => ({}))
vi.mock('@xterm/addon-fit', () => ({
  FitAddon: class {
    fit() {}
  },
}))
// jsdom has no ResizeObserver; the terminal only uses it to refit on container resize.
globalThis.ResizeObserver = class {
  observe() {}
  unobserve() {}
  disconnect() {}
}

import { AvailableAgentsSection } from '@/components/panels/agent-ops/AvailableAgentsSection'
import type { BackendCmd } from '@/protocol/types'
import { initialData, useStore } from '@/store'
import { RingBuffer } from '@/store/ring-buffer'
import { commandFinished, resetActions, setActionTimeout } from '@/store/actions'

let sent: BackendCmd[]
let nextId: number

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
  setActionTimeout()
  sent = []
  nextId = 200
  useStore.setState({
    request: (cmd) => {
      sent.push(cmd)
      return Promise.resolve(nextId++)
    },
  })
})

function statusArrived(enabled: boolean, running: boolean, connectionCommand = '/usr/bin/fathom --only-mcp') {
  useStore.setState({
    mcp: {
      ...useStore.getState().mcp,
      loaded: true,
      enabled,
      running,
      connectionCommand,
    },
  })
}

/** Put the demo chat into one of its four fixture states, without any backend. */
function demoChatIs(
  status: 'idle' | 'running' | 'ended' | 'not-detected',
  extra: { instructions?: string; sessionId?: number; reason?: string } = {},
) {
  useStore.setState({
    mcp: {
      ...useStore.getState().mcp,
      demoChat: { status, chunks: new RingBuffer<string>(), rev: 0, ...extra },
    },
  })
}

describe('AvailableAgentsSection', () => {
  it('only reads on mount, and never asks the backend to enable anything', () => {
    render(<AvailableAgentsSection />)
    expect(sent).toEqual([{ type: 'LoadMcpServerStatus' }, { type: 'AgentGetAuthMode' }])
    expect(sent.some((c) => c.type === 'SetMcpServerEnabled')).toBe(false)
  })

  it('starts off: unchecked, nothing listening, and no connection snippet', () => {
    render(<AvailableAgentsSection />)
    expect(screen.getByTestId('mcp-server-toggle')).not.toBeChecked()
    expect(screen.getByTestId('mcp-server-state')).toHaveTextContent('Off')
    expect(screen.queryByTestId('mcp-connection-command')).toBeNull()
    expect(screen.queryByTestId('mcp-running-indicator')).toBeNull()
  })

  it('toggling on dispatches SetMcpServerEnabled with enabled: true', async () => {
    const user = userEvent.setup()
    render(<AvailableAgentsSection />)
    await user.click(screen.getByTestId('mcp-server-toggle'))

    expect(sent).toContainEqual({ type: 'SetMcpServerEnabled', data: { enabled: true } })
    commandFinished(nextId - 1, { ok: true, error: null })
  })

  it('toggling off dispatches SetMcpServerEnabled with enabled: false', async () => {
    const user = userEvent.setup()
    statusArrived(true, true)
    render(<AvailableAgentsSection />)
    await user.click(screen.getByTestId('mcp-server-toggle'))

    expect(sent).toContainEqual({ type: 'SetMcpServerEnabled', data: { enabled: false } })
    commandFinished(nextId - 1, { ok: true, error: null })
  })

  it('shows the running indicator and connection snippet once the backend reports running', () => {
    statusArrived(true, true, '/usr/local/bin/fathom --only-mcp')
    render(<AvailableAgentsSection />)
    expect(screen.getByTestId('mcp-server-toggle')).toBeChecked()
    expect(screen.getByTestId('mcp-running-indicator')).toHaveTextContent('Running')
    expect(screen.getByTestId('mcp-connection-command')).toHaveTextContent(
      '/usr/local/bin/fathom --only-mcp',
    )
  })

  /** A start that failed must not read as healthy: `running` comes from the backend, not the toggle. */
  it('reports enabled-but-not-running distinctly from running', () => {
    statusArrived(true, false)
    render(<AvailableAgentsSection />)
    expect(screen.getByTestId('mcp-server-state')).toHaveTextContent('Enabled, but not running.')
    expect(screen.queryByTestId('mcp-running-indicator')).toBeNull()
  })

  it('surfaces a backend refusal verbatim', async () => {
    const user = userEvent.setup()
    render(<AvailableAgentsSection />)
    await user.click(screen.getByTestId('mcp-server-toggle'))
    commandFinished(nextId - 1, { ok: false, error: 'The MCP server is disabled in this process.' })
    expect(await screen.findByTestId('mcp-error')).toHaveTextContent(
      'The MCP server is disabled in this process.',
    )
  })

  it('summarizes the assistant auth mode read-only, and never shows a key', () => {
    useStore.setState({
      agentAuth: { mode: { type: 'Ollama', data: { endpoint: 'http://localhost:11434', model: 'mistral' } } },
    })
    render(<AvailableAgentsSection />)
    const summary = screen.getByTestId('mcp-agent-auth-summary')
    expect(summary).toHaveTextContent('Ollama')
    expect(summary).toHaveTextContent('mistral')
    expect(summary).toHaveTextContent('http://localhost:11434')
  })

  it('summarizes a BYOK Anthropic mode without naming the key', () => {
    useStore.setState({
      agentAuth: {
        mode: { type: 'Anthropic', data: { key_configured: true } },
        revealedKey: 'sk-ant-secret',
      },
    })
    render(<AvailableAgentsSection />)
    const summary = screen.getByTestId('mcp-agent-auth-summary')
    expect(summary).toHaveTextContent('Anthropic')
    expect(screen.queryByText(/sk-ant-secret/)).toBeNull()
  })

  /**
   * T040/FR-008: the summary is a *read* of the same `agentAuth.mode` slice `AuthSetup.tsx`
   * owns. When that slice is already populated — the Ask AI tab's setup ran earlier in the
   * session — mounting this section must cost no second credential lookup: the only command it
   * sends is its own status read.
   */
  it('reuses an already-loaded auth mode without a second credential lookup', () => {
    useStore.setState({
      agentAuth: { mode: { type: 'Anthropic', data: { key_configured: true } } },
    })
    render(<AvailableAgentsSection />)

    expect(sent).toEqual([{ type: 'LoadMcpServerStatus' }])
    expect(sent.some((c) => c.type === 'AgentGetAuthMode')).toBe(false)
    expect(screen.getByTestId('mcp-agent-auth-summary')).toHaveTextContent('Anthropic')
  })

  it('reads the auth mode once when nothing has loaded it yet, and never writes it', async () => {
    const user = userEvent.setup()
    render(<AvailableAgentsSection />)

    expect(sent.filter((c) => c.type === 'AgentGetAuthMode')).toHaveLength(1)
    // No edit affordance here — changing the mode stays AuthSetup's job (spec non-goal).
    expect(screen.queryByTestId('agent-ops-mode-selector')).toBeNull()
    await user.click(screen.getByTestId('mcp-server-toggle'))
    expect(sent.some((c) => c.type === 'AgentSetAuthMode')).toBe(false)
    commandFinished(nextId - 1, { ok: true, error: null })
  })

  /** US3 acceptance scenario 2: after a restart the persisted preference is on-screen before
   * the first status broadcast arrives, so the operator never sees a false "off". */
  it('shows the persisted preference until the first status broadcast lands', () => {
    useStore.setState({ prefs: { loaded: true, expandedGroups: [], mcpStdioEnabled: true } })
    render(<AvailableAgentsSection />)
    expect(screen.getByTestId('mcp-server-toggle')).toBeChecked()
  })

  /** …and the backend's live word always wins over the persisted preference. */
  it('lets the backend status override the persisted preference', () => {
    useStore.setState({ prefs: { loaded: true, expandedGroups: [], mcpStdioEnabled: true } })
    statusArrived(false, false)
    render(<AvailableAgentsSection />)
    expect(screen.getByTestId('mcp-server-toggle')).not.toBeChecked()
  })
})

/**
 * T045: the demo chat's three honest states (FR-009/FR-010/FR-011), each driven purely by
 * fixture state — no subprocess, no backend.
 */
describe('AvailableAgentsSection demo chat', () => {
  it('server off: explains the server must be on first, and offers no launch', () => {
    statusArrived(false, false)
    render(<AvailableAgentsSection />)

    expect(screen.getByTestId('mcp-demo-chat-server-off')).toHaveTextContent(
      'Turn the MCP server on first',
    )
    expect(screen.queryByTestId('mcp-demo-chat-launch')).toBeNull()
    expect(screen.queryByTestId('mcp-demo-chat-terminal')).toBeNull()
  })

  it('not detected: shows the backend\u2019s setup instructions and no button', () => {
    statusArrived(true, true)
    demoChatIs('not-detected', { instructions: 'Install Claude Code, then reopen this panel.' })
    render(<AvailableAgentsSection />)

    expect(screen.getByTestId('mcp-demo-chat-not-detected')).toHaveTextContent(
      'Install Claude Code, then reopen this panel.',
    )
    expect(screen.queryByTestId('mcp-demo-chat-launch')).toBeNull()
  })

  it('detected: offers the launch button, which sends StartMcpDemoChat', async () => {
    const user = userEvent.setup()
    const localSent: BackendCmd[] = []
    useStore.setState({ send: (cmd) => localSent.push(cmd) })
    statusArrived(true, true)
    render(<AvailableAgentsSection />)

    const launch = screen.getByTestId('mcp-demo-chat-launch')
    expect(launch).toHaveTextContent('Launch example chat')
    expect(screen.queryByTestId('mcp-demo-chat-not-detected')).toBeNull()
    expect(screen.queryByTestId('mcp-demo-chat-terminal')).toBeNull()

    await user.click(launch)
    expect(localSent).toContainEqual({ type: 'StartMcpDemoChat' })
  })

  it('running: shows the terminal and a stop control', () => {
    statusArrived(true, true)
    demoChatIs('running', { sessionId: 1 })
    render(<AvailableAgentsSection />)

    expect(screen.getByTestId('mcp-demo-chat-terminal')).toBeInTheDocument()
    expect(screen.getByTestId('mcp-demo-chat-stop')).toBeInTheDocument()
  })

  it('ended: reports the backend\u2019s reason verbatim and offers a relaunch', () => {
    statusArrived(true, true)
    demoChatIs('ended', { sessionId: 1, reason: 'unknown option --mcp-config' })
    render(<AvailableAgentsSection />)

    expect(screen.getByTestId('mcp-demo-chat-ended')).toHaveTextContent(
      'unknown option --mcp-config',
    )
    expect(screen.getByTestId('mcp-demo-chat-launch')).toHaveTextContent('Relaunch example chat')
  })
})
