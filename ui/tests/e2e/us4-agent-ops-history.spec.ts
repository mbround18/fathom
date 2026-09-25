/**
 * US4 (013-agent-sdk-ops, Phase 6/P2 — persistence/history): FR-017/AS3.
 *
 * This suite drives the fake in-browser backend (`window.__fathomFake`), the same harness
 * `us1-agent-ops-ask.spec.ts`/`us3-agent-ops-confirm.spec.ts` use. That fake backend is
 * entirely client-side and has no disk-backed state of its own, so it cannot simulate the real
 * Rust backend's `~/.fathom/agent_conversations.json` persistence surviving an actual app
 * restart — there is no existing pattern anywhere in this e2e suite for that (grepped for
 * specs exercising `credentials.json`-backed reload: none exist either). What *is* provable at
 * this layer, and covered below:
 *
 * (a) a conversation with a message and an executed action, once loaded (as
 *     `AgentConversationUpdated` would echo it back from the persisted file on
 *     `AgentOpenConversation`), renders correctly and nothing is auto-replayed — remounting the
 *     Agent tab sends only `AgentOpenConversation`, never an automatic `AgentAsk` (FR-017/AS3).
 * (b) "Clear history" is confirmation-gated (a misclick does not wipe history).
 * (c) confirming "Clear history" sends `AgentClearConversation`, and once the backend echoes
 *     back an empty conversation, the visible history is gone and stays gone across a remount.
 *
 * Cross-restart durability of the write to `agent_conversations.json` itself (load-on-startup,
 * save-on-mutation, corrupt/missing-file fallback) is proven only by the Rust unit tests in
 * `src/backend/agent/persistence.rs`, not by this suite.
 */
import { expect, test } from '@playwright/test'
import type { Page } from '@playwright/test'

import type { AgentConversation, AgentObjectRef } from '../../src/protocol/types'
import { DETAIL_POD } from '../fixtures'
import { batch, globalRoute, read } from '../fixtures/builders'
import { clearSent, confirmDialog, emit, inspector, open, row, sent, sentTypes } from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`

const OBJECT: AgentObjectRef = {
  kind: 'Pod',
  namespace: DETAIL_POD.namespace,
  name: DETAIL_POD.name,
}

const CONVERSATION: AgentConversation = {
  messages: [
    {
      id: 'msg-1',
      author: 'Operator',
      text: 'Why is this pod restarting?',
      grounding: [],
      suggested_actions: [],
      created_at: '2026-09-22T00:00:00Z',
    },
    {
      id: 'msg-2',
      author: 'Assistant',
      text: 'This pod was OOMKilled twice in the last hour.',
      grounding: [{ type: 'Events' }],
      suggested_actions: [
        {
          id: 'sugg-1',
          action_kind: 'restart',
          target: OBJECT,
          params: {},
          rationale: 'Clears the current OOMKilled crash loop for a fresh attempt.',
          status: 'Reviewed',
        },
      ],
      created_at: '2026-09-22T00:00:05Z',
    },
  ],
  executed_actions: [
    {
      id: 'exec-1',
      suggested_action_id: 'sugg-1',
      command_summary: `Restart ${key(DETAIL_POD)}`,
      outcome: { type: 'Success' },
      executed_at: '2026-09-22T00:00:10Z',
    },
  ],
}

async function openAgentTab(page: Page) {
  await row(page, key(DETAIL_POD)).click()
  const insp = inspector(page)
  await expect(insp).toBeVisible()
  await clearSent(page)
  await insp.getByRole('tab', { name: 'Ask AI' }).click()
  await expect.poll(() => sentTypes(page)).toContain('AgentOpenConversation')
  return insp
}

test.describe('US4 agent ops: conversation history', () => {
  test('(a) a loaded conversation renders, and reopening never auto-replays a question', async ({
    page,
  }) => {
    await open(page)
    const insp = await openAgentTab(page)

    await emit(
      page,
      batch([
        read(10, globalRoute('agent'), {
          type: 'AgentConversationUpdated',
          data: { object: OBJECT, conversation: CONVERSATION },
        }),
      ]),
    )

    const messages = insp.getByTestId('agent-ops-message')
    await expect(messages).toHaveCount(2)
    await expect(messages.nth(0)).toContainText('Why is this pod restarting?')
    await expect(messages.nth(1)).toContainText('OOMKilled twice')
    await expect(insp.getByTestId('agent-ops-suggested-action-outcome')).toContainText('succeeded')

    // Leave the Agent tab and come back — the only thing sent on remount is
    // `AgentOpenConversation`, never an automatic `AgentAsk` (FR-017/AS3: history is inert).
    await clearSent(page)
    await insp.getByRole('tab', { name: 'Details' }).click()
    await insp.getByRole('tab', { name: 'Ask AI' }).click()
    await expect.poll(() => sentTypes(page)).toContain('AgentOpenConversation')
    expect(await sentTypes(page)).not.toContain('AgentAsk')
  })

  test('(b)+(c) "Clear history" is confirmation-gated and permanently removes history', async ({
    page,
  }) => {
    await open(page)
    const insp = await openAgentTab(page)
    await emit(
      page,
      batch([
        read(10, globalRoute('agent'), {
          type: 'AgentConversationUpdated',
          data: { object: OBJECT, conversation: CONVERSATION },
        }),
      ]),
    )
    await expect(insp.getByTestId('agent-ops-message')).toHaveCount(2)

    // A misclick alone must not wipe history: clicking "Clear history" only opens a confirm
    // step, and Cancel leaves the conversation untouched.
    await clearSent(page)
    await insp.getByTestId('agent-ops-clear-history').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    expect(await sentTypes(page)).toEqual([])

    await dialog.getByTestId('confirm-cancel').click()
    await expect(dialog).toBeHidden()
    expect(await sentTypes(page)).toEqual([])
    await expect(insp.getByTestId('agent-ops-message')).toHaveCount(2)

    // Confirming sends AgentClearConversation, and the empty conversation the backend echoes
    // back removes the visible history.
    await insp.getByTestId('agent-ops-clear-history').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect
      .poll(() => sent(page))
      .toContainEqual({ type: 'AgentClearConversation', data: { object: OBJECT } })

    await emit(
      page,
      batch([
        read(11, globalRoute('agent'), {
          type: 'AgentConversationUpdated',
          data: { object: OBJECT, conversation: { messages: [], executed_actions: [] } },
        }),
      ]),
    )
    await expect(insp.getByTestId('agent-ops-empty')).toBeVisible()
    await expect(insp.getByTestId('agent-ops-message')).toHaveCount(0)

    // It does not reappear on remount either — nothing auto-restores what was just cleared.
    await clearSent(page)
    await insp.getByRole('tab', { name: 'Details' }).click()
    await insp.getByRole('tab', { name: 'Ask AI' }).click()
    await expect.poll(() => sentTypes(page)).toContain('AgentOpenConversation')
    await expect(insp.getByTestId('agent-ops-empty')).toBeVisible()
    await expect(insp.getByTestId('agent-ops-message')).toHaveCount(0)
  })
})
