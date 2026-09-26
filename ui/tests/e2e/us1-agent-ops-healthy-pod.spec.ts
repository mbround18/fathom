/**
 * T026 (013-agent-sdk-ops, US1 acceptance scenario 3): asking about a healthy pod gets an answer
 * that says so, with no suggested-action card — and, like every read-only question, no
 * cluster-mutating command.
 */
import { expect, test } from '@playwright/test'

import type { AgentConversation, AgentObjectRef } from '../../src/protocol/types'
import { DETAIL_POD } from '../fixtures'
import { batch, globalRoute, nsRoute, read } from '../fixtures/builders'
import { podDetails } from '../fixtures/gen-pods'
import {
  clearSent,
  clusterMutations,
  emit,
  finish,
  inspector,
  nextRequestId,
  open,
  row,
  sentTypes,
} from './helpers'

const KEY = `${DETAIL_POD.namespace}/${DETAIL_POD.name}`
const OBJECT: AgentObjectRef = {
  kind: 'Pod',
  namespace: DETAIL_POD.namespace,
  name: DETAIL_POD.name,
}

test.describe('US1 agent ops: a healthy pod', () => {
  test('the answer says the pod appears healthy and suggests no action', async ({ page }) => {
    await open(page)
    await row(page, KEY).click()
    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect.poll(() => sentTypes(page)).toContain('SelectPod')
    await clearSent(page)

    // `podDetails()` is a running, ready container with no restarts.
    await emit(
      page,
      batch([
        read(60, nsRoute('kind-a', 'workloads/pods', DETAIL_POD.namespace), {
          type: 'PodDetailsUpdated',
          data: podDetails(DETAIL_POD),
        }),
      ]),
    )

    await insp.getByRole('tab', { name: 'Ask AI' }).click()
    await expect.poll(() => sentTypes(page)).toContain('AgentOpenConversation')

    const question = 'Is anything wrong with this pod?'
    await insp.getByTestId('agent-ops-question-input').fill(question)
    const askId = await nextRequestId(page)
    await insp.getByTestId('agent-ops-submit').click()
    await expect.poll(() => sentTypes(page)).toContain('AgentAsk')
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
          text: 'This pod appears healthy: its container is running and ready, with no restarts and no warning events.',
          grounding: [{ type: 'Events' }, { type: 'ContainerStatuses' }],
          suggested_actions: [],
          created_at: '2026-09-22T00:00:05Z',
        },
      ],
      executed_actions: [],
    }
    await emit(
      page,
      batch([
        read(61, globalRoute('agent'), {
          type: 'AgentConversationUpdated',
          data: { object: OBJECT, conversation },
        }),
      ]),
    )

    const messages = insp.getByTestId('agent-ops-message')
    await expect(messages).toHaveCount(2)
    await expect(messages.nth(1)).toContainText('appears healthy')
    await expect(insp.getByTestId('agent-ops-loading')).toHaveCount(0)
    await expect(insp.getByTestId('agent-ops-suggested-action-card')).toHaveCount(0)

    expect(await clusterMutations(page)).toEqual([])
  })
})
