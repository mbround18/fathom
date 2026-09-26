/**
 * T042 (013-agent-sdk-ops, US3 acceptance scenario 5): confirming a suggested action the
 * operator's credentials may not perform fails exactly like the manual path — the API server's
 * forbidden message verbatim in the same error toast, the failure recorded against the
 * suggestion, and no automatic retry (constitution Principle I; FR-008).
 */
import { expect, test } from '@playwright/test'

import type { AgentConversation, AgentObjectRef } from '../../src/protocol/types'
import { batch, globalRoute, read } from '../fixtures/builders'
import { genDeployments } from '../fixtures/gen-pods'
import {
  DEPLOYMENTS,
  actionToast,
  clearSent,
  confirmDialog,
  emit,
  finish,
  inspector,
  nextRequestId,
  open,
  row,
  sent,
  sentTypes,
  showList,
} from './helpers'

const found = genDeployments('kind-a').find((d) => d.desired === 2) ?? genDeployments('kind-a')[0]
if (!found) throw new Error('fixture: at least one deployment')
const D = found
const KEY = `${D.namespace}/${D.name}`
const OBJECT: AgentObjectRef = { kind: 'Deployment', namespace: D.namespace, name: D.name }
const FORBIDDEN = `deployments.apps "${D.name}" is forbidden: User "system:serviceaccount:fathom-dev:viewer" cannot patch resource "deployments/scale" in API group "apps" in the namespace "${D.namespace}"`

const conversation: AgentConversation = {
  messages: [
    {
      id: 'msg-1',
      author: 'Assistant',
      text: 'This deployment is under-provisioned for current load.',
      grounding: [{ type: 'Events' }],
      suggested_actions: [
        {
          id: 'sugg-1',
          action_kind: 'scale',
          target: OBJECT,
          params: { replicas: 5 },
          rationale: 'Traffic has doubled in the last 10 minutes.',
          status: 'Proposed',
        },
      ],
      created_at: '2026-09-22T00:00:00Z',
    },
  ],
  executed_actions: [],
}

test('a forbidden confirmed action surfaces the verbatim error, is recorded, and is not retried', async ({
  page,
}) => {
  await open(page)
  await showList(page, DEPLOYMENTS, 'Deployments')
  await row(page, KEY, DEPLOYMENTS).click()
  const insp = inspector(page)
  await expect(insp).toBeVisible()
  await insp.getByRole('tab', { name: 'Ask AI' }).click()
  await expect.poll(() => sentTypes(page)).toContain('AgentOpenConversation')
  await emit(
    page,
    batch([
      read(10, globalRoute('agent'), {
        type: 'AgentConversationUpdated',
        data: { object: OBJECT, conversation },
      }),
    ]),
  )

  const card = insp.getByTestId('agent-ops-suggested-action-card')
  await card.getByTestId('agent-ops-suggested-action-review').click()
  const dialog = confirmDialog(page)
  await expect(dialog).toBeVisible()

  await clearSent(page)
  const scaleId = await nextRequestId(page)
  await dialog.getByTestId('confirm-submit').click()
  await expect.poll(() => sentTypes(page)).toEqual(['ScaleDeployment'])

  await finish(page, scaleId, false, FORBIDDEN)

  // The same toast the manual scale path shows, carrying the API server's message verbatim.
  const toast = actionToast(page, 'error')
  await expect(toast).toContainText('Scale failed')
  await expect(toast).toContainText(FORBIDDEN)

  // The failure is recorded against the suggestion, verbatim — never paraphrased.
  await expect
    .poll(() => sent(page))
    .toContainEqual({
      type: 'AgentRecordExecutedAction',
      data: {
        object: OBJECT,
        suggestion_id: 'sugg-1',
        command_summary: `Scale ${KEY}`,
        outcome: { type: 'Error', data: { message: FORBIDDEN } },
      },
    })

  // No automatic retry: exactly one mutating command was ever sent.
  await page.waitForTimeout(500)
  expect((await sentTypes(page)).filter((t) => t === 'ScaleDeployment')).toHaveLength(1)
})
