/**
 * US1 (013-agent-sdk-ops, Phase 3/P1 MVP): open the "Ask AI" tab for a pod, ask a question, and
 * see the assistant's reply (with a suggested-action card) appear once `AgentConversationUpdated`
 * arrives. Mirrors `us2-helm-upgrade.spec.ts`'s pattern of driving the fake backend directly
 * (`emit`/`sent`) rather than a real Anthropic/Ollama call, which this suite cannot make.
 */
import { expect, test } from '@playwright/test'

import type { AgentConversation, AgentObjectRef } from '../../src/protocol/types'
import { DETAIL_POD } from '../fixtures'
import { batch, globalRoute, read } from '../fixtures/builders'
import { clearSent, emit, finish, inspector, nextRequestId, open, row, sent } from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`

const OBJECT: AgentObjectRef = { kind: 'Pod', namespace: DETAIL_POD.namespace, name: DETAIL_POD.name }

test.describe('US1 agent ops: ask the assistant', () => {
  test('open the Agent tab, ask a question, and see the response appear', async ({ page }) => {
    await open(page)
    await row(page, key(DETAIL_POD)).click()

    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await clearSent(page)

    await insp.getByRole('tab', { name: 'Ask AI' }).click()
    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toContain('AgentOpenConversation')

    // The conversation starts empty until `AgentOpenConversation`'s reply arrives.
    await expect(insp.getByTestId('agent-ops-empty')).toBeVisible()

    const question = 'Why is this pod restarting?'
    await insp.getByTestId('agent-ops-question-input').fill(question)
    const askRequestId = await nextRequestId(page)
    await insp.getByTestId('agent-ops-submit').click()

    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toContain('AgentAsk')
    const ask = (await sent(page)).find((c) => c.type === 'AgentAsk')
    expect(ask).toEqual({ type: 'AgentAsk', data: { object: OBJECT, question } })

    await expect(insp.getByTestId('agent-ops-loading')).toBeVisible()

    // The backend command itself always succeeds for `AgentAsk` (a provider error becomes the
    // assistant's reply text, not a command failure) — `CommandFinished{ok:true}` then the
    // conversation update, exactly as `src/backend/cmd/agent.rs::handle_ask` emits it.
    await finish(page, askRequestId, true)

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
          text: 'This pod was OOMKilled twice in the last hour.',
          grounding: [{ type: 'Events' }, { type: 'ContainerStatuses' }],
          suggested_actions: [
            {
              id: 'sugg-1',
              action_kind: 'restart',
              target: OBJECT,
              params: {},
              rationale: 'Clears the current OOMKilled crash loop for a fresh attempt.',
              status: 'Proposed',
            },
          ],
          created_at: '2026-09-22T00:00:05Z',
        },
      ],
      executed_actions: [],
    }
    await emit(
      page,
      batch([
        read(10, globalRoute('agent'), { type: 'AgentConversationUpdated', data: { object: OBJECT, conversation } }),
      ]),
    )

    await expect(insp.getByTestId('agent-ops-loading')).toHaveCount(0)
    const messages = insp.getByTestId('agent-ops-message')
    await expect(messages).toHaveCount(2)
    await expect(messages.nth(0)).toContainText(question)
    await expect(messages.nth(1)).toContainText('OOMKilled twice')

    const suggestion = insp.getByTestId('agent-ops-suggested-action')
    await expect(suggestion).toBeVisible()
    await expect(suggestion).toContainText('restart')
    await expect(suggestion).toContainText('Clears the current OOMKilled crash loop')
  })
})
