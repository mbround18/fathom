/**
 * US2 (013-agent-sdk-ops, Phase 4/P1): the assistant auth-mode setup form. Mirrors
 * `us1-agent-ops-ask.spec.ts`'s pattern of driving the fake backend directly (`emit`/`sent`)
 * since this suite cannot make a real Anthropic/Ollama call. Covers: opening the "Ask AI" tab's
 * auth settings, setting BYOK mode, that the reveal-key result only appears after the explicit
 * click (never before), and a simulated auth-failure error state shown verbatim.
 */
import { expect, test } from '@playwright/test'

import { DETAIL_POD } from '../fixtures'
import { batch, system } from '../fixtures/builders'
import { clearSent, emit, finish, inspector, nextRequestId, open, row, sent } from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`

test.describe('US2 agent ops: assistant auth setup', () => {
  test('set BYOK mode, reveal the key only after an explicit click, and see a verbatim auth error', async ({
    page,
  }) => {
    await open(page)
    await row(page, key(DETAIL_POD)).click()

    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await clearSent(page)

    await insp.getByRole('tab', { name: 'Ask AI' }).click()
    await insp.getByTestId('agent-ops-auth-toggle').click()

    const authSetup = insp.getByTestId('agent-ops-auth-setup')
    await expect(authSetup).toBeVisible()

    // Opening the form itself asks for the current mode — never reveals a key on its own.
    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toContain('AgentGetAuthMode')
    await expect(authSetup.getByTestId('agent-ops-reveal-key')).toHaveCount(0)
    await expect(authSetup.getByTestId('agent-ops-revealed-key')).toHaveCount(0)

    // --- Set BYOK mode ---
    await authSetup.getByTestId('agent-ops-byok-key-input').fill('sk-ant-test-key')
    const setRequestId = await nextRequestId(page)
    await authSetup.getByTestId('agent-ops-auth-submit').click()

    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toContain('AgentSetAuthMode')
    const setCmd = (await sent(page)).find((c) => c.type === 'AgentSetAuthMode')
    expect(setCmd).toEqual({
      type: 'AgentSetAuthMode',
      data: { mode: { type: 'Anthropic', data: { key: 'sk-ant-test-key' } } },
    })

    await finish(page, setRequestId, true)
    await emit(
      page,
      batch([
        system(20, {
          type: 'AgentAuthModeUpdated',
          data: { mode: { type: 'Anthropic', data: { key_configured: true } } },
        }),
      ]),
    )
    await expect(authSetup.getByTestId('agent-ops-auth-saved')).toBeVisible()

    // --- Reveal key: gated behind an explicit click, never shown before it ---
    const revealButton = authSetup.getByTestId('agent-ops-reveal-key')
    await expect(revealButton).toBeVisible()
    await expect(authSetup.getByTestId('agent-ops-revealed-key')).toHaveCount(0)

    const revealRequestId = await nextRequestId(page)
    await revealButton.click()
    await expect.poll(async () => (await sent(page)).map((c) => c.type)).toContain('AgentRevealKey')

    // Still not shown until the backend's reply actually arrives.
    await expect(authSetup.getByTestId('agent-ops-revealed-key')).toHaveCount(0)

    await emit(
      page,
      batch([system(21, { type: 'AgentKeyRevealed', data: { key: 'sk-ant-test-key' } })]),
    )
    await finish(page, revealRequestId, true)

    await expect(authSetup.getByTestId('agent-ops-revealed-key')).toHaveText('sk-ant-test-key')

    await authSetup.getByTestId('agent-ops-hide-key').click()
    await expect(authSetup.getByTestId('agent-ops-revealed-key')).toHaveCount(0)

    // --- Simulated auth failure surfaces verbatim ---
    await authSetup.getByTestId('agent-ops-byok-key-input').fill('sk-ant-bad-key')
    const failRequestId = await nextRequestId(page)
    await authSetup.getByTestId('agent-ops-auth-submit').click()
    await finish(
      page,
      failRequestId,
      false,
      'Anthropic rejected the API key (invalid or revoked). Anthropic API error 401 Unauthorized: {"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}',
    )

    await expect(authSetup.getByTestId('agent-ops-auth-error')).toHaveText(
      'Anthropic rejected the API key (invalid or revoked). Anthropic API error 401 Unauthorized: {"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}',
    )
  })
})
