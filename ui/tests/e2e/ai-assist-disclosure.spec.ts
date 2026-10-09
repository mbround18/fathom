/**
 * The one-time disclosure before the first Ask AI request to a hosted provider: pod logs,
 * events, resource status and the question leave the machine, scrubbing is best-effort. Driven
 * against the fake backend: `AiProviderStatus` picks the provider, `UiConfigLoaded` replays a
 * persisted acceptance, and `SaveUiConfig` (opted into via `sent(page, true)`) proves the
 * acceptance is persisted through the same path as every other UI preference.
 */
import { expect, test, type Page } from '@playwright/test'

import type { Provider, UiConfig } from '../../src/protocol/types'
import { DETAIL_POD } from '../fixtures'
import { batch, system } from '../fixtures/builders'
import {
  clearSent,
  confirmDialog,
  emit,
  finish,
  inspector,
  open,
  requestIdOf,
  row,
  sent,
  sentTypes,
} from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`

function providerStatus(active: Provider) {
  return {
    type: 'AiProviderStatus' as const,
    data: {
      statuses: (['github', 'claude', 'openai', 'ollama'] as const).map((provider) => ({
        provider,
        state: { type: provider === active ? ('Connected' as const) : ('NotConfigured' as const) },
        account_label: null,
        last_checked_at: null,
      })),
      active,
      offline: false,
      ollama_model: active === 'ollama' ? 'llama3.2' : null,
    },
  }
}

async function openAgentTab(page: Page, active: Provider, ui?: UiConfig) {
  await open(page)
  const envelopes = [system(2, providerStatus(active))]
  if (ui) {
    envelopes.push(
      system(3, {
        type: 'UiConfigLoaded',
        data: { ui, expanded_groups: ['Workloads'], mcp_stdio_enabled: false },
      }),
    )
  }
  await emit(page, batch(envelopes))
  await row(page, key(DETAIL_POD)).click()
  const insp = inspector(page)
  await expect(insp).toBeVisible()
  await insp.getByRole('tab', { name: 'Ask AI' }).click()
  await expect.poll(() => sentTypes(page)).toContain('AgentOpenConversation')
  await clearSent(page)
  return insp
}

test.describe('Ask AI hosted-provider disclosure', () => {
  test('shown once for a hosted provider: cancel sends nothing, send persists and never re-prompts', async ({
    page,
  }) => {
    const insp = await openAgentTab(page, 'github')
    const input = insp.getByTestId('agent-ops-question-input')

    await input.fill('Why is this pod restarting?')
    await insp.getByTestId('agent-ops-submit').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await expect(dialog.getByTestId('agent-ops-disclosure')).toContainText('pod logs')
    await expect(dialog.getByTestId('agent-ops-disclosure')).toContainText('best-effort')

    // Cancel: nothing leaves, the question is still there.
    await dialog.getByTestId('confirm-cancel').click()
    await expect(dialog).toBeHidden()
    expect(await sentTypes(page, true)).toEqual([])
    await expect(input).toHaveValue('Why is this pod restarting?')

    // Send: the acceptance is persisted and the question goes out.
    await insp.getByTestId('agent-ops-submit').click()
    await expect(dialog).toBeVisible()
    await dialog.getByTestId('confirm-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('AgentAsk')
    const askId = await requestIdOf(page, 'AgentAsk')
    const saved = (await sent(page, true)).find((c) => c.type === 'SaveUiConfig')
    expect(saved?.type === 'SaveUiConfig' && saved.data.ui.aiDisclosureAccepted).toBe(true)

    // A second question is not gated again (the input re-enables once the first ask finishes).
    await finish(page, askId, true)
    await clearSent(page)
    await input.fill('And now?')
    await insp.getByTestId('agent-ops-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('AgentAsk')
    await expect(confirmDialog(page)).toHaveCount(0)
  })

  test('a persisted acceptance is honoured on the next start', async ({ page }) => {
    const insp = await openAgentTab(page, 'claude', {
      schema: 1,
      theme: { mode: 'dark', accent: 'cyan', effects: 'on', glitch: false },
      density: {},
      layouts: {},
      aiDisclosureAccepted: true,
    })
    await insp.getByTestId('agent-ops-question-input').fill('Why?')
    await insp.getByTestId('agent-ops-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('AgentAsk')
    await expect(confirmDialog(page)).toHaveCount(0)
  })

  test('local Ollama never prompts', async ({ page }) => {
    const insp = await openAgentTab(page, 'ollama')
    await insp.getByTestId('agent-ops-question-input').fill('Why?')
    await insp.getByTestId('agent-ops-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('AgentAsk')
    await expect(confirmDialog(page)).toHaveCount(0)
    expect(await sentTypes(page, true)).not.toContain('SaveUiConfig')
  })
})
