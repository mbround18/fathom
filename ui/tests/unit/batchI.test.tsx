import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { AiAssistSettings } from '@/features/ai-assist/AiAssistSettings'
import { RoleMapEditor } from '@/features/identity/RoleMapEditor'
import type { BackendCmd } from '@/protocol/types'
import { initialData, useStore } from '@/store'
import { resetActions, setActionTimeout } from '@/store/actions'

let sent: BackendCmd[]
const send = vi.fn()

beforeEach(() => {
  useStore.setState(initialData())
  resetActions()
  setActionTimeout()
  sent = []
  send.mockReset()
  useStore.setState({
    send,
    request: (cmd) => {
      sent.push(cmd)
      return Promise.resolve(1)
    },
  })
})

const st = (state: object) => ({
  provider: 'github' as const,
  state,
  account_label: null,
  last_checked_at: null,
})

function loadAi(state: object) {
  useStore.setState({
    ai: {
      loaded: true,
      statuses: [st(state)] as never,
      active: null,
      offline: false,
      ollamaModel: null,
    },
  })
}

describe('Ollama endpoint and file store', () => {
  it('tests and saves an Ollama endpoint', async () => {
    loadAi({ type: 'NotConfigured' })
    render(<AiAssistSettings />)
    sent.length = 0
    await userEvent.type(screen.getByTestId('ollama-endpoint-input'), 'http://10.0.0.5:11434')
    await userEvent.click(screen.getByTestId('ollama-endpoint-test'))
    await userEvent.click(screen.getByTestId('ollama-allow-lan'))
    await userEvent.click(screen.getByTestId('ollama-endpoint-save'))
    expect(sent).toEqual([
      { type: 'AiOllamaProbe', data: { endpoint: 'http://10.0.0.5:11434' } },
      { type: 'AiSetOllamaEndpoint', data: { endpoint: 'http://10.0.0.5:11434', allow_lan: true } },
    ])
  })

  it('hides the file store when a keyring exists', () => {
    loadAi({ type: 'NotConfigured' })
    render(<AiAssistSettings />)
    expect(screen.queryByTestId('ai-file-store')).toBeNull()
  })

  it('requires explicit confirmation before enabling the file store', async () => {
    loadAi({ type: 'Blocked', data: { reason: 'store_unavailable' } })
    render(<AiAssistSettings />)
    sent.length = 0
    const btn = screen.getByTestId('ai-file-store-enable')
    expect(btn).toBeDisabled()
    await userEvent.click(screen.getByTestId('ai-file-store-confirm'))
    await userEvent.click(btn)
    expect(sent).toEqual([{ type: 'AiEnableFileStore', data: { confirm: true } }])
  })
})

describe('RoleMapEditor', () => {
  it('saves edited groups in bootstrap state', async () => {
    render(<RoleMapEditor />)
    await userEvent.type(screen.getByTestId('oidc-role-admins_group'), ' ops ')
    await userEvent.click(screen.getByTestId('oidc-role-map-save'))
    expect(send).toHaveBeenCalledWith({
      type: 'OidcSetRoleMap',
      data: { role_map: { viewers_group: '', editors_group: '', admins_group: 'ops' } },
    })
  })

  it('is read-only for non-admins', () => {
    useStore.setState({
      identity: {
        ...useStore.getState().identity,
        config: {
          issuers: [],
          contexts: [],
          role_map: { viewers_group: 'v', editors_group: 'e', admins_group: 'a' },
        },
      },
    })
    render(<RoleMapEditor />)
    expect(screen.getByTestId('oidc-role-map-readonly')).toBeTruthy()
    expect(screen.getByTestId('oidc-role-map-save')).toBeDisabled()
  })
})
