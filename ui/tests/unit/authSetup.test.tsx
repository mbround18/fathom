/**
 * `AuthSetup` (013-agent-sdk-ops, US2/P1): the global assistant auth-mode form. Mirrors
 * `agentPanel.test.tsx`'s conventions (direct `request`/`commandFinished` driving, no real
 * transport). Covers: mode selection, BYOK key submit dispatches `AgentSetAuthMode`,
 * reveal-key dispatches `AgentRevealKey` and shows the result only after that explicit click
 * (never before), and verbatim backend error display.
 */
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it } from 'vitest'

import { AuthSetup } from '@/components/panels/agent-ops/AuthSetup'
import type { BackendCmd } from '@/protocol/types'
import { initialData, useStore } from '@/store'
import { commandFinished, resetActions, setActionTimeout } from '@/store/actions'

let sent: BackendCmd[]
let nextId: number

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
  setActionTimeout()
  sent = []
  nextId = 100
  useStore.setState({
    request: (cmd) => {
      sent.push(cmd)
      return Promise.resolve(nextId++)
    },
  })
})

describe('AuthSetup', () => {
  it('requests the current auth mode on mount and defaults to the Anthropic mode selector', () => {
    render(<AuthSetup />)
    expect(sent).toEqual([{ type: 'AgentGetAuthMode' }])
    expect(screen.getByTestId('agent-ops-mode-anthropic')).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByTestId('agent-ops-byok-key-input')).toBeInTheDocument()
  })

  it('selecting Ollama shows the endpoint/model inputs instead of the key input', async () => {
    const user = userEvent.setup()
    render(<AuthSetup />)
    await user.click(screen.getByTestId('agent-ops-mode-ollama'))
    expect(screen.queryByTestId('agent-ops-byok-key-input')).toBeNull()
    expect(screen.getByTestId('agent-ops-ollama-endpoint-input')).toBeInTheDocument()
    expect(screen.getByTestId('agent-ops-ollama-model-input')).toBeInTheDocument()
  })

  it('submitting a BYOK key dispatches AgentSetAuthMode with that key', async () => {
    const user = userEvent.setup()
    render(<AuthSetup />)
    await user.type(screen.getByTestId('agent-ops-byok-key-input'), 'sk-ant-test-key')
    await user.click(screen.getByTestId('agent-ops-auth-submit'))

    expect(sent).toContainEqual({
      type: 'AgentSetAuthMode',
      data: { mode: { type: 'Anthropic', data: { key: 'sk-ant-test-key' } } },
    })

    const requestId = nextId - 1
    commandFinished(requestId, { ok: true, error: null })
    expect(await screen.findByTestId('agent-ops-auth-saved')).toBeInTheDocument()
  })

  it('submitting Ollama settings dispatches AgentSetAuthMode with endpoint/model', async () => {
    const user = userEvent.setup()
    render(<AuthSetup />)
    await user.click(screen.getByTestId('agent-ops-mode-ollama'))
    const endpointInput = screen.getByTestId('agent-ops-ollama-endpoint-input')
    await user.clear(endpointInput)
    await user.type(endpointInput, 'http://localhost:11434')
    const modelInput = screen.getByTestId('agent-ops-ollama-model-input')
    await user.clear(modelInput)
    await user.type(modelInput, 'mistral')
    await user.click(screen.getByTestId('agent-ops-auth-submit'))

    expect(sent).toContainEqual({
      type: 'AgentSetAuthMode',
      data: { mode: { type: 'Ollama', data: { endpoint: 'http://localhost:11434', model: 'mistral' } } },
    })
  })

  it('does not show a reveal-key control until a key is configured', () => {
    render(<AuthSetup />)
    expect(screen.queryByTestId('agent-ops-reveal-key')).toBeNull()
    expect(screen.queryByTestId('agent-ops-revealed-key')).toBeNull()
  })

  it('reveal-key dispatches AgentRevealKey and shows the result only after the click', async () => {
    useStore.setState((s) => ({
      agentAuth: { ...s.agentAuth, mode: { type: 'Anthropic', data: { key_configured: true } } },
    }))
    const user = userEvent.setup()
    render(<AuthSetup />)

    // Not shown before the click, even though a key is configured and the mode is loaded.
    expect(screen.queryByTestId('agent-ops-revealed-key')).toBeNull()

    await user.click(screen.getByTestId('agent-ops-reveal-key'))
    expect(sent).toContainEqual({ type: 'AgentRevealKey' })

    // Still not shown until the backend's reply actually arrives.
    expect(screen.queryByTestId('agent-ops-revealed-key')).toBeNull()

    const requestId = nextId - 1
    useStore.setState((s) => ({ agentAuth: { ...s.agentAuth, revealedKey: 'sk-ant-the-real-key' } }))
    commandFinished(requestId, { ok: true, error: null })

    expect(await screen.findByTestId('agent-ops-revealed-key')).toHaveTextContent(
      'sk-ant-the-real-key',
    )

    await user.click(screen.getByTestId('agent-ops-hide-key'))
    expect(screen.queryByTestId('agent-ops-revealed-key')).toBeNull()
  })

  it('shows the backend error verbatim when AgentSetAuthMode fails', async () => {
    const user = userEvent.setup()
    render(<AuthSetup />)
    await user.type(screen.getByTestId('agent-ops-byok-key-input'), 'sk-ant-bad')
    await user.click(screen.getByTestId('agent-ops-auth-submit'))

    const requestId = nextId - 1
    commandFinished(requestId, {
      ok: false,
      error: 'Anthropic rejected the API key (invalid or revoked). Anthropic API error 401 Unauthorized: invalid x-api-key',
    })

    expect(await screen.findByTestId('agent-ops-auth-error')).toHaveTextContent(
      'Anthropic rejected the API key (invalid or revoked). Anthropic API error 401 Unauthorized: invalid x-api-key',
    )
  })

  it('shows a verbatim error when reveal-key fails', async () => {
    useStore.setState((s) => ({
      agentAuth: { ...s.agentAuth, mode: { type: 'Anthropic', data: { key_configured: true } } },
    }))
    const user = userEvent.setup()
    render(<AuthSetup />)
    await user.click(screen.getByTestId('agent-ops-reveal-key'))

    const requestId = nextId - 1
    commandFinished(requestId, { ok: false, error: 'No key configured to reveal.' })

    expect(await screen.findByTestId('agent-ops-auth-error')).toHaveTextContent(
      'No key configured to reveal.',
    )
    expect(screen.queryByTestId('agent-ops-revealed-key')).toBeNull()
  })

  it('shows an error state when request() itself rejects', async () => {
    useStore.setState({ request: () => Promise.reject(new Error('Not connected to backend')) })
    const user = userEvent.setup()
    render(<AuthSetup />)
    await user.type(screen.getByTestId('agent-ops-byok-key-input'), 'sk-ant-test')
    await user.click(screen.getByTestId('agent-ops-auth-submit'))

    expect(await screen.findByTestId('agent-ops-auth-error')).toHaveTextContent(
      'Not connected to backend',
    )
  })
})
