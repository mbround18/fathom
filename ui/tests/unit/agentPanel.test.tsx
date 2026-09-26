/**
 * `AgentPanel` (013-agent-sdk-ops, US1/P1 MVP): the empty state, sending `AgentOpenConversation`
 * on mount and `AgentAsk` on submit, rendering messages/suggested actions from the store's
 * `agentConversations` slice, and the loading/error states around the `AgentAsk` round trip. All
 * testids asserted here are the `agent-ops-` prefixed ones this feature introduces.
 */
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { AgentPanel } from '@/components/panels/agent-ops/AgentPanel'
import type { AgentConversation, AgentObjectRef, BackendCmd } from '@/protocol/types'
import { initialData, useStore } from '@/store'
import { commandFinished, resetActions, setActionTimeout } from '@/store/actions'
import { agentConversationKey } from '@/store/handlers/agent'

const TARGET: AgentObjectRef = { kind: 'Pod', namespace: 'default', name: 'api-0' }
const KEY = agentConversationKey(TARGET.kind, TARGET.namespace, TARGET.name)

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

describe('AgentPanel', () => {
  it('opens the conversation on mount and shows the empty state', () => {
    render(<AgentPanel object={TARGET} />)
    expect(sent).toEqual([{ type: 'AgentOpenConversation', data: { object: TARGET } }])
    expect(screen.getByTestId('agent-ops-empty')).toBeInTheDocument()
    expect(screen.queryByTestId('agent-ops-message')).toBeNull()
  })

  it('submitting a question sends AgentAsk and shows a loading state until it resolves', async () => {
    const user = userEvent.setup()
    render(<AgentPanel object={TARGET} />)
    await user.type(screen.getByTestId('agent-ops-question-input'), 'Why is this pod crashing?')
    await user.click(screen.getByTestId('agent-ops-submit'))

    expect(sent).toContainEqual({
      type: 'AgentAsk',
      data: { object: TARGET, question: 'Why is this pod crashing?' },
    })
    expect(await screen.findByTestId('agent-ops-loading')).toBeInTheDocument()

    const askId = nextId - 1
    commandFinished(askId, { ok: true, error: null })
    await vi.waitFor(() => expect(screen.queryByTestId('agent-ops-loading')).toBeNull())
  })

  it('renders operator and assistant messages, including suggested-action cards', () => {
    const conversation: AgentConversation = {
      messages: [
        {
          id: 'msg-1',
          author: 'Operator',
          text: 'Why is this pod crashing?',
          grounding: [],
          suggested_actions: [],
          created_at: '2026-09-22T00:00:00Z',
        },
        {
          id: 'msg-2',
          author: 'Assistant',
          text: 'It looks OOMKilled.',
          grounding: [{ type: 'Events' }],
          suggested_actions: [
            {
              id: 'sugg-1',
              action_kind: 'restart',
              target: TARGET,
              params: {},
              rationale: 'Pod is crash-looping on OOMKilled',
              status: 'Proposed',
            },
          ],
          created_at: '2026-09-22T00:00:05Z',
        },
      ],
      executed_actions: [],
    }
    useStore.setState((s) => ({
      agentConversations: { ...s.agentConversations, [KEY]: conversation },
    }))
    render(<AgentPanel object={TARGET} />)

    const messages = screen.getAllByTestId('agent-ops-message')
    expect(messages).toHaveLength(2)
    expect(messages[0]).toHaveTextContent('Why is this pod crashing?')
    expect(messages[1]).toHaveTextContent('It looks OOMKilled.')

    const action = screen.getByTestId('agent-ops-suggested-action-card')
    expect(action).toHaveTextContent('restart')
    expect(action).toHaveTextContent('Pod is crash-looping on OOMKilled')
  })

  it('shows the grounding sources of each answer, flagging a truncated log tail (FR-012/FR-018)', () => {
    const conversation: AgentConversation = {
      messages: [
        {
          id: 'msg-1',
          author: 'Operator',
          text: 'Why?',
          grounding: [],
          suggested_actions: [],
          created_at: '2026-09-22T00:00:00Z',
        },
        {
          id: 'msg-2',
          author: 'Assistant',
          text: 'CrashLoopBackOff: the container exits 1 on start.',
          grounding: [
            { type: 'Events' },
            { type: 'ContainerStatuses' },
            { type: 'Logs', data: { truncated: true } },
          ],
          suggested_actions: [],
          created_at: '2026-09-22T00:00:05Z',
        },
      ],
      executed_actions: [],
    }
    useStore.setState((s) => ({
      agentConversations: { ...s.agentConversations, [KEY]: conversation },
    }))
    render(<AgentPanel object={TARGET} />)

    const lists = screen.getAllByTestId('agent-ops-grounding-sources')
    expect(lists).toHaveLength(1)
    const chips = screen.getAllByTestId('agent-ops-grounding-source').map((c) => c.textContent)
    expect(chips).toEqual(['Events', 'Container statuses', 'Logs (truncated)'])
  })

  it('shows an error state when the AgentAsk command fails', async () => {
    const user = userEvent.setup()
    render(<AgentPanel object={TARGET} />)
    await user.type(screen.getByTestId('agent-ops-question-input'), 'Why is this pod crashing?')
    await user.click(screen.getByTestId('agent-ops-submit'))

    const askId = nextId - 1
    commandFinished(askId, { ok: false, error: 'The assistant is not configured yet.' })
    await vi.waitFor(() =>
      expect(screen.getByTestId('agent-ops-error')).toHaveTextContent(
        'The assistant is not configured yet.',
      ),
    )
    expect(screen.queryByTestId('agent-ops-loading')).toBeNull()
  })

  it('shows an error state when request() itself rejects', async () => {
    useStore.setState({ request: () => Promise.reject(new Error('Not connected to backend')) })
    const user = userEvent.setup()
    render(<AgentPanel object={TARGET} />)
    await user.type(screen.getByTestId('agent-ops-question-input'), 'Why is this pod crashing?')
    await user.click(screen.getByTestId('agent-ops-submit'))

    expect(await screen.findByTestId('agent-ops-error')).toHaveTextContent(
      'Not connected to backend',
    )
  })

  it('"Clear history" is disabled with no messages, and does nothing without confirming', () => {
    render(<AgentPanel object={TARGET} />)

    expect(screen.getByTestId('agent-ops-clear-history')).toBeDisabled()
  })

  it('"Clear history" asks for confirmation, and sends AgentClearConversation only on confirm', async () => {
    const conversation: AgentConversation = {
      messages: [
        {
          id: 'msg-1',
          author: 'Operator',
          text: 'Why is this pod crashing?',
          grounding: [],
          suggested_actions: [],
          created_at: '2026-09-22T00:00:00Z',
        },
      ],
      executed_actions: [],
    }
    useStore.setState((s) => ({
      agentConversations: { ...s.agentConversations, [KEY]: conversation },
    }))
    const user = userEvent.setup()
    render(<AgentPanel object={TARGET} />)

    await user.click(screen.getByTestId('agent-ops-clear-history'))
    expect(screen.getByTestId('confirm-dialog')).toBeInTheDocument()
    // Not sent yet: only AgentOpenConversation (on mount) so far.
    expect(sent).toEqual([{ type: 'AgentOpenConversation', data: { object: TARGET } }])

    await user.click(screen.getByTestId('confirm-cancel'))
    expect(screen.queryByTestId('confirm-dialog')).toBeNull()
    expect(sent).toEqual([{ type: 'AgentOpenConversation', data: { object: TARGET } }])

    await user.click(screen.getByTestId('agent-ops-clear-history'))
    await user.click(screen.getByTestId('confirm-submit'))
    expect(screen.queryByTestId('confirm-dialog')).toBeNull()
    expect(sent).toContainEqual({ type: 'AgentClearConversation', data: { object: TARGET } })
  })

  describe('auth gating (FR-003/FR-013)', () => {
    it('with no assistant configured shows a setup prompt and blocks question submission', async () => {
      useStore.setState((s) => ({ agentAuth: { ...s.agentAuth, mode: { type: 'Unconfigured' } } }))
      const user = userEvent.setup()
      render(<AgentPanel object={TARGET} />)

      expect(screen.getByTestId('agent-ops-setup-required')).toBeInTheDocument()
      const input = screen.getByTestId('agent-ops-question-input')
      expect(input).toBeDisabled()
      expect(screen.getByTestId('agent-ops-submit')).toBeDisabled()

      await user.type(input, 'Why is this pod crashing?{Enter}')
      expect(sent.map((c) => c.type)).not.toContain('AgentAsk')

      await user.click(screen.getByTestId('agent-ops-setup-open'))
      expect(screen.getByTestId('agent-ops-auth-setup')).toBeInTheDocument()
    })

    it('the disable toggle prevents question submission', async () => {
      useStore.setState((s) => ({
        agentAuth: { ...s.agentAuth, mode: { type: 'Anthropic', data: { key_configured: true } } },
      }))
      const user = userEvent.setup()
      render(<AgentPanel object={TARGET} />)
      const input = screen.getByTestId('agent-ops-question-input')
      expect(input).toBeEnabled()

      await user.click(screen.getByTestId('agent-ops-auth-toggle'))
      await user.click(screen.getByTestId('agent-ops-mode-disabled'))
      await user.click(screen.getByTestId('agent-ops-auth-submit'))
      expect(sent).toContainEqual({
        type: 'AgentSetAuthMode',
        data: { mode: { type: 'Disabled' } },
      })

      // The backend reports "no active provider" back as Unconfigured.
      commandFinished(nextId - 1, { ok: true, error: null })
      useStore.setState((s) => ({ agentAuth: { ...s.agentAuth, mode: { type: 'Unconfigured' } } }))

      await vi.waitFor(() => expect(input).toBeDisabled())
      expect(screen.getByTestId('agent-ops-submit')).toBeDisabled()
      await user.type(input, 'still there?{Enter}')
      expect(sent.map((c) => c.type)).not.toContain('AgentAsk')
    })

    it('a provider the 013 mode cannot represent (038 GitHub) is not blocked', () => {
      useStore.setState((s) => ({
        agentAuth: { ...s.agentAuth, mode: { type: 'Unconfigured' } },
        ai: { ...s.ai, loaded: true, active: 'github' },
      }))
      render(<AgentPanel object={TARGET} />)
      expect(screen.queryByTestId('agent-ops-setup-required')).toBeNull()
      expect(screen.getByTestId('agent-ops-question-input')).toBeEnabled()
    })
  })
})
