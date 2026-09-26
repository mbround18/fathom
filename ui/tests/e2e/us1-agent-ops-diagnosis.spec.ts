/**
 * T025 (013-agent-sdk-ops, US1 acceptance scenario 1): a pod in `CrashLoopBackOff` — ask the
 * assistant why, and see a diagnosis that cites the data it was grounded in (FR-012), with a
 * suggested action that stays a suggestion. The whole scenario — selecting the pod, asking,
 * receiving the answer with its suggestion — sends zero cluster-mutating commands (FR-007/SC-003).
 *
 * The fake backend scripts no AI replies, so the answer is emitted the way
 * `src/backend/cmd/agent.rs::handle_ask` delivers it: `CommandFinished{ok:true}` for the
 * `AgentAsk`, then `AgentConversationUpdated`.
 */
import { expect, test } from '@playwright/test'

import type { AgentConversation, AgentObjectRef, PodDetails } from '../../src/protocol/types'
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
  sent,
  sentTypes,
} from './helpers'

const KEY = `${DETAIL_POD.namespace}/${DETAIL_POD.name}`
const OBJECT: AgentObjectRef = {
  kind: 'Pod',
  namespace: DETAIL_POD.namespace,
  name: DETAIL_POD.name,
}

function crashLooping(): PodDetails {
  const base = podDetails(DETAIL_POD)
  const [app] = base.containers_lifecycle
  if (!app) throw new Error('fixture: podDetails has an app container')
  return {
    ...base,
    containers_lifecycle: [
      {
        ...app,
        ready: false,
        state: {
          phase: 'waiting',
          reason: 'CrashLoopBackOff',
          exit_code: null,
          started_at: null,
          finished_at: null,
        },
        restart_count: 7,
        last_state: {
          phase: 'terminated',
          reason: 'Error',
          exit_code: 1,
          started_at: '2026-09-22T00:00:00Z',
          finished_at: '2026-09-22T00:00:02Z',
        },
      },
    ],
  }
}

test.describe('US1 agent ops: diagnose a crash-looping pod', () => {
  test('asks why a CrashLoopBackOff pod is failing and gets a grounded diagnosis — no mutation', async ({
    page,
  }) => {
    await open(page)
    await row(page, KEY).click()
    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect.poll(() => sentTypes(page)).toContain('SelectPod')
    // Everything from here on is the diagnosis scenario proper.
    await clearSent(page)

    await emit(
      page,
      batch([
        read(60, nsRoute('kind-a', 'workloads/pods', DETAIL_POD.namespace), {
          type: 'PodDetailsUpdated',
          data: crashLooping(),
        }),
      ]),
    )

    await insp.getByRole('tab', { name: 'Ask AI' }).click()
    await expect.poll(() => sentTypes(page)).toContain('AgentOpenConversation')

    const question = 'Why is this pod in CrashLoopBackOff?'
    await insp.getByTestId('agent-ops-question-input').fill(question)
    const askId = await nextRequestId(page)
    await insp.getByTestId('agent-ops-submit').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'AgentAsk'))
      .toEqual({ type: 'AgentAsk', data: { object: OBJECT, question } })

    await finish(page, askId, true)
    const conversation: AgentConversation = {
      messages: [
        {
          id: 'msg-1',
          author: 'Operator',
          text: question,
          grounding: [],
          suggested_actions: [],
          created_at: '2026-09-22T00:00:10Z',
        },
        {
          id: 'msg-2',
          author: 'Assistant',
          text: 'The container is in CrashLoopBackOff after 7 restarts: it exits with code 1 two seconds after starting. The log tail shows it cannot reach its database.',
          grounding: [
            { type: 'Events' },
            { type: 'ContainerStatuses' },
            { type: 'Logs', data: { truncated: true } },
          ],
          suggested_actions: [
            {
              id: 'sugg-1',
              action_kind: 'restart',
              target: OBJECT,
              params: {},
              rationale: 'Once the database is reachable, a fresh start clears the back-off.',
              status: 'Proposed',
            },
          ],
          created_at: '2026-09-22T00:00:15Z',
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
    const answer = messages.nth(1)
    await expect(answer).toContainText('CrashLoopBackOff after 7 restarts')
    await expect(answer).toContainText('exits with code 1')

    // FR-012: the answer says which data it drew on; FR-018: the log tail was truncated.
    await expect(answer.getByTestId('agent-ops-grounding-source')).toHaveText([
      'Events',
      'Container statuses',
      'Logs (truncated)',
    ])

    // The suggestion is only a suggestion until an operator confirms it.
    const card = insp.getByTestId('agent-ops-suggested-action-card')
    await expect(card).toBeVisible()
    await expect(card.getByTestId('agent-ops-suggested-action-status')).toHaveText('Proposed')

    // FR-007/SC-003: nothing in the diagnosis flow touched the cluster.
    expect(await clusterMutations(page)).toEqual([])
    expect(await sentTypes(page)).not.toContain('AgentRecordExecutedAction')
  })
})
