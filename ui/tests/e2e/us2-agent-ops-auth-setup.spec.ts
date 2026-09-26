/**
 * T032 (013-agent-sdk-ops, US2 acceptance scenarios 1-5): the assistant's auth gate and modes,
 * driven through the fake backend (`emit`/`sent`/`finish`) since this suite makes no real
 * Anthropic/Ollama call. The backend reports "no active provider" (disabled or never set up) as
 * `Unconfigured` — see `src/backend/agent/credentials.rs`.
 *
 * Limitation: scenario 3 ("Ollama-only makes no hosted-provider network request") is proven here
 * at the command boundary — the UI sends only the Ollama mode and never a hosted-provider
 * command. The network half (no request to a hosted API when Ollama is active) is covered by
 * the Rust tests (`ai::tests`, `agent::ollama`), where requests can actually be observed.
 */
import { expect, test, type Page } from '@playwright/test'

import type { AgentAuthMode, AgentConversation, AgentObjectRef } from '../../src/protocol/types'
import { DETAIL_POD } from '../fixtures'
import { batch, globalRoute, read, system } from '../fixtures/builders'
import {
  clearSent,
  clusterMutations,
  emit,
  finish,
  inspector,
  nextRequestId,
  open,
  row,
  sent,
  sentTypes,
} from './helpers'

const KEY = `${DETAIL_POD.namespace}/${DETAIL_POD.name}`
const OBJECT: AgentObjectRef = {
  kind: 'Pod',
  namespace: DETAIL_POD.namespace,
  name: DETAIL_POD.name,
}
const HOSTED_PROVIDER_COMMANDS = ['AiProviderLoginStart', 'AiProviderSubmitApiKey']

async function openAgentTab(page: Page) {
  await open(page)
  await row(page, KEY).click()
  const insp = inspector(page)
  await expect(insp).toBeVisible()
  await clearSent(page)
  await insp.getByRole('tab', { name: 'Ask AI' }).click()
  await expect.poll(() => sentTypes(page)).toContain('AgentOpenConversation')
  return insp
}

async function emitMode(page: Page, seq: number, mode: AgentAuthMode) {
  await emit(page, batch([system(seq, { type: 'AgentAuthModeUpdated', data: { mode } })]))
}

test.describe('US2 agent ops: auth gate and modes', () => {
  test('(1) with no auth configured the panel prompts for setup and blocks questions', async ({
    page,
  }) => {
    const insp = await openAgentTab(page)
    await emitMode(page, 20, { type: 'Unconfigured' })

    await expect(insp.getByTestId('agent-ops-setup-required')).toBeVisible()
    const input = insp.getByTestId('agent-ops-question-input')
    await expect(input).toBeDisabled()
    await expect(insp.getByTestId('agent-ops-submit')).toBeDisabled()
    expect(await sentTypes(page)).not.toContain('AgentAsk')

    await insp.getByTestId('agent-ops-setup-open').click()
    const authSetup = insp.getByTestId('agent-ops-auth-setup')
    await expect(authSetup).toBeVisible()
    await expect(authSetup.getByTestId('agent-ops-mode-anthropic')).toBeVisible()
    await expect(authSetup.getByTestId('agent-ops-mode-ollama')).toBeVisible()
  })

  test('(2) a BYOK key is sent masked, saved, and unblocks questions', async ({ page }) => {
    const insp = await openAgentTab(page)
    await emitMode(page, 20, { type: 'Unconfigured' })
    await insp.getByTestId('agent-ops-setup-open').click()
    const authSetup = insp.getByTestId('agent-ops-auth-setup')

    const keyInput = authSetup.getByTestId('agent-ops-byok-key-input')
    await expect(keyInput).toHaveAttribute('type', 'password')
    await keyInput.fill('sk-ant-e2e-key')
    const setId = await nextRequestId(page)
    await authSetup.getByTestId('agent-ops-auth-submit').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'AgentSetAuthMode'))
      .toEqual({
        type: 'AgentSetAuthMode',
        data: { mode: { type: 'Anthropic', data: { key: 'sk-ant-e2e-key' } } },
      })
    await finish(page, setId, true)
    await emitMode(page, 21, { type: 'Anthropic', data: { key_configured: true } })
    await expect(authSetup.getByTestId('agent-ops-auth-saved')).toBeVisible()
    // The key never comes back unless the operator explicitly asks to reveal it.
    await expect(authSetup.getByTestId('agent-ops-revealed-key')).toHaveCount(0)

    await expect(insp.getByTestId('agent-ops-setup-required')).toHaveCount(0)
    await insp.getByTestId('agent-ops-question-input').fill('Why is this pod restarting?')
    await insp.getByTestId('agent-ops-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('AgentAsk')
  })

  test('(3) Ollama-only mode sends only the Ollama config, never a hosted-provider command', async ({
    page,
  }) => {
    const insp = await openAgentTab(page)
    await emitMode(page, 20, { type: 'Unconfigured' })
    await insp.getByTestId('agent-ops-setup-open').click()
    const authSetup = insp.getByTestId('agent-ops-auth-setup')

    await authSetup.getByTestId('agent-ops-mode-ollama').click()
    await authSetup.getByTestId('agent-ops-ollama-endpoint-input').fill('http://127.0.0.1:11434')
    await authSetup.getByTestId('agent-ops-ollama-model-input').fill('llama3.2')
    const setId = await nextRequestId(page)
    await authSetup.getByTestId('agent-ops-auth-submit').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'AgentSetAuthMode'))
      .toEqual({
        type: 'AgentSetAuthMode',
        data: {
          mode: { type: 'Ollama', data: { endpoint: 'http://127.0.0.1:11434', model: 'llama3.2' } },
        },
      })
    await finish(page, setId, true)
    await emitMode(page, 21, {
      type: 'Ollama',
      data: { endpoint: 'http://127.0.0.1:11434', model: 'llama3.2' },
    })

    await insp.getByTestId('agent-ops-question-input').fill('Why is this pod restarting?')
    await insp.getByTestId('agent-ops-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('AgentAsk')

    const types = await sentTypes(page)
    for (const hosted of HOSTED_PROVIDER_COMMANDS) expect(types).not.toContain(hosted)
    expect(await clusterMutations(page)).toEqual([])
  })

  test('(4) an unreachable Ollama endpoint is reported verbatim', async ({ page }) => {
    const insp = await openAgentTab(page)
    await emitMode(page, 20, {
      type: 'Ollama',
      data: { endpoint: 'http://127.0.0.1:11434', model: 'llama3.2' },
    })

    const question = 'Why is this pod restarting?'
    await insp.getByTestId('agent-ops-question-input').fill(question)
    const askId = await nextRequestId(page)
    await insp.getByTestId('agent-ops-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('AgentAsk')

    // `handle_ask` turns a provider error into the assistant's reply text, verbatim.
    const error =
      'Could not reach local Ollama endpoint http://127.0.0.1:11434: error sending request for url (http://127.0.0.1:11434/api/chat): Connection refused (os error 111)'
    await finish(page, askId, true)
    const conversation: AgentConversation = {
      messages: [
        {
          id: 'msg-1',
          author: 'Operator',
          text: question,
          grounding: [],
          suggested_actions: [],
          created_at: '2026-09-22T00:00:00Z',
        },
        {
          id: 'msg-2',
          author: 'Assistant',
          text: error,
          grounding: [],
          suggested_actions: [],
          created_at: '2026-09-22T00:00:01Z',
        },
      ],
      executed_actions: [],
    }
    await emit(
      page,
      batch([
        read(30, globalRoute('agent'), {
          type: 'AgentConversationUpdated',
          data: { object: OBJECT, conversation },
        }),
      ]),
    )
    await expect(insp.getByTestId('agent-ops-message').nth(1)).toContainText(error)
    await expect(insp.getByTestId('agent-ops-suggested-action-card')).toHaveCount(0)
  })

  test('(5) disabling the assistant blocks further questions', async ({ page }) => {
    const insp = await openAgentTab(page)
    await emitMode(page, 20, { type: 'Anthropic', data: { key_configured: true } })
    const input = insp.getByTestId('agent-ops-question-input')
    await expect(input).toBeEnabled()

    await insp.getByTestId('agent-ops-auth-toggle').click()
    const authSetup = insp.getByTestId('agent-ops-auth-setup')
    await authSetup.getByTestId('agent-ops-mode-disabled').click()
    const setId = await nextRequestId(page)
    await authSetup.getByTestId('agent-ops-auth-submit').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'AgentSetAuthMode'))
      .toEqual({ type: 'AgentSetAuthMode', data: { mode: { type: 'Disabled' } } })
    await finish(page, setId, true)
    await emitMode(page, 21, { type: 'Unconfigured' })

    await expect(input).toBeDisabled()
    await expect(insp.getByTestId('agent-ops-submit')).toBeDisabled()
    expect(await sentTypes(page)).not.toContain('AgentAsk')
  })
})
