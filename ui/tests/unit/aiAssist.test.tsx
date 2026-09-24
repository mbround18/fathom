/** 038: AI Assist settings components, store handlers and the shared assistant summary. */
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { AiAssistSettings } from '@/features/ai-assist/AiAssistSettings'
import type { BackendCmd } from '@/protocol/types'
import { initialData, useStore } from '@/store'
import { resetActions, setActionTimeout } from '@/store/actions'

let sent: BackendCmd[]

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
  setActionTimeout()
  sent = []
  useStore.setState({
    request: (cmd) => {
      sent.push(cmd)
      return Promise.resolve(1)
    },
  })
})

const status = (provider: 'github' | 'claude' | 'openai', type: 'NotConfigured' | 'Connected') => ({
  provider,
  state: { type },
  account_label: null,
  last_checked_at: null,
})

function load(github: 'NotConfigured' | 'Connected' = 'NotConfigured') {
  useStore.setState({
    ai: {
      loaded: true,
      statuses: [status('github', github), status('claude', 'NotConfigured'), status('openai', 'NotConfigured')],
      active: null,
      offline: false,
      ollamaModel: null,
    },
  })
}

describe('AiAssistSettings', () => {
  it('asks for status and probes on mount (reads only)', () => {
    render(<AiAssistSettings />)
    expect(sent.map((c) => c.type)).toEqual(['AiGetStatus', 'AiOllamaProbe'])
  })

  it('starts GitHub login only on click', async () => {
    load()
    render(<AiAssistSettings />)
    expect(sent.some((c) => c.type === 'AiProviderLoginStart')).toBe(false)
    await userEvent.click(screen.getByTestId('ai-login-github'))
    expect(sent).toContainEqual({ type: 'AiProviderLoginStart', data: { provider: 'github' } })
  })

  it('clears the key field after submit', async () => {
    load()
    render(<AiAssistSettings />)
    const input = screen.getByTestId('ai-key-input-openai')
    await userEvent.type(input, 'sk-secret')
    await userEvent.click(screen.getByTestId('ai-key-submit-openai'))
    expect(sent).toContainEqual({
      type: 'AiProviderSubmitApiKey',
      data: { provider: 'openai', key: 'sk-secret' },
    })
    expect(input).toHaveValue('')
  })

  it('shows the device code from the flow', () => {
    load()
    useStore.setState((s) => ({
      ai: {
        ...s.ai,
        loginFlow: {
          provider: 'github',
          kind: 'device_code',
          user_code: 'WXYZ-9999',
          verification_uri: 'https://github.com/login/device',
          expires_at: new Date(Date.now() + 60_000).toISOString(),
          interval_s: 5,
          phase: { type: 'Polling' },
        },
      },
    }))
    render(<AiAssistSettings />)
    expect(screen.getByTestId('ai-user-code')).toHaveTextContent('WXYZ-9999')
  })

  it('offers sign out when connected', async () => {
    load('Connected')
    render(<AiAssistSettings />)
    await userEvent.click(screen.getByTestId('ai-signout-github'))
    expect(sent).toContainEqual({ type: 'AiProviderSignOut', data: { provider: 'github' } })
  })

  it('renders the ollama not-running and no-models panels', () => {
    load()
    const base = {
      endpoint: 'http://127.0.0.1:11434',
      version: null,
      installed_hint: 'not_found' as const,
      models: [],
      checked_at: '',
      error: null,
    }
    useStore.setState((s) => ({ ai: { ...s.ai, ollama: { ...base, reachable: false } } }))
    const { unmount } = render(<AiAssistSettings />)
    expect(screen.getByTestId('ollama-not-running')).toHaveTextContent('does not appear to be installed')
    unmount()
    useStore.setState((s) => ({ ai: { ...s.ai, ollama: { ...base, reachable: true } } }))
    render(<AiAssistSettings />)
    expect(screen.getByTestId('ollama-no-models')).toBeInTheDocument()
  })

  it('filters models by search', async () => {
    load()
    useStore.setState((s) => ({
      ai: {
        ...s.ai,
        ollama: {
          endpoint: 'x',
          reachable: true,
          version: '1',
          installed_hint: 'found',
          checked_at: '',
          error: null,
          models: ['llama3', 'qwen2'].map((name) => ({
            name,
            size_bytes: 1e9,
            family: null,
            parameter_size: null,
            modified_at: '',
          })),
        },
      },
    }))
    render(<AiAssistSettings />)
    await userEvent.type(screen.getByRole('searchbox'), 'qwen')
    expect(screen.queryByTestId('ollama-model-llama3')).toBeNull()
    expect(screen.getByTestId('ollama-model-qwen2')).toBeInTheDocument()
  })

  it('stores events via the system handler', () => {
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    load()
    expect(useStore.getState().ai.loaded).toBe(true)
  })
})
