/**
 * US3 (013-agent-sdk-ops, Phase 5/P1 — THE SAFETY-CRITICAL PHASE): confirming a suggested
 * action runs through the app's existing confirm-dialog + mutation path (constitution
 * Principle I), never a shortcut. Mirrors `us2-helm-upgrade.spec.ts`'s confirm-dialog/diff
 * conventions. Covers: (a) a suggestion shown in the panel, (b) Review opens the existing
 * `ConfirmDialog` with a diff, (c) confirming sends the real `ScaleDeployment` and then
 * `AgentRecordExecutedAction` with a success outcome, (d) declining sends no mutating command,
 * only `AgentDeclineSuggestedAction`, (e) an unmatched `action_kind` is informational-only with
 * no Review button.
 */
import { expect, test, type Page } from '@playwright/test'

import type { AgentConversation, AgentObjectRef } from '../../src/protocol/types'
import { genDeployments } from '../fixtures/gen-pods'
import { batch, globalRoute, read } from '../fixtures/builders'
import {
  DEPLOYMENTS,
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
const DEP_NAMESPACE = D.namespace
const DEP_NAME = D.name
const KEY = `${DEP_NAMESPACE}/${DEP_NAME}`
const OBJECT: AgentObjectRef = { kind: 'Deployment', namespace: DEP_NAMESPACE, name: DEP_NAME }

async function selectDeploymentAndOpenAgentTab(page: Page) {
  await open(page)
  await showList(page, DEPLOYMENTS, 'Deployments')
  await row(page, KEY, DEPLOYMENTS).click()
  const insp = inspector(page)
  await expect(insp).toBeVisible()
  await clearSent(page)
  await insp.getByRole('tab', { name: 'Ask AI' }).click()
  await expect.poll(() => sentTypes(page)).toContain('AgentOpenConversation')
  return insp
}

function conversationWith(
  action_kind: string,
  params: unknown = {},
  status: 'Proposed' | 'Reviewed' | 'Declined' = 'Proposed',
): AgentConversation {
  return {
    messages: [
      {
        id: 'msg-1',
        author: 'Assistant',
        text: 'This deployment is under-provisioned for current load.',
        grounding: [{ type: 'Events' }],
        suggested_actions: [
          {
            id: 'sugg-1',
            action_kind,
            target: { kind: 'Deployment', namespace: D.namespace, name: D.name },
            params,
            rationale: 'Traffic has doubled in the last 10 minutes.',
            status,
          },
        ],
        created_at: '2026-09-22T00:00:00Z',
      },
    ],
    executed_actions: [],
  }
}

async function emitConversation(page: Page, seq: number, conversation: AgentConversation) {
  await emit(
    page,
    batch([
      read(seq, globalRoute('agent'), {
        type: 'AgentConversationUpdated',
        data: { object: OBJECT, conversation },
      }),
    ]),
  )
}

test.describe('US3 agent ops: confirm/execute a suggested action', () => {
  test('(a)+(b) suggestion appears, Review opens ConfirmDialog with a diff', async ({ page }) => {
    const insp = await selectDeploymentAndOpenAgentTab(page)
    await emitConversation(page, 10, conversationWith('scale', { replicas: 5 }))

    const card = insp.getByTestId('agent-ops-suggested-action-card')
    await expect(card).toBeVisible()
    await expect(card).toContainText('scale')
    await expect(card).toContainText(KEY)
    await expect(card.getByTestId('agent-ops-suggested-action-status')).toHaveText('Proposed')

    await card.getByTestId('agent-ops-suggested-action-review').click()

    // Proposed → Reviewed is sent as pure bookkeeping alongside opening the dialog.
    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toContain('AgentReviewSuggestedAction')

    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await expect(dialog.getByTestId('confirm-item')).toContainText(KEY)
  })

  test('(c) confirming sends the real ScaleDeployment, then AgentRecordExecutedAction(Success)', async ({
    page,
  }) => {
    const insp = await selectDeploymentAndOpenAgentTab(page)
    await emitConversation(page, 10, conversationWith('scale', { replicas: 5 }))

    const card = insp.getByTestId('agent-ops-suggested-action-card')
    await card.getByTestId('agent-ops-suggested-action-review').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()

    await clearSent(page)
    const scaleRequestId = await nextRequestId(page)
    await dialog.getByTestId('confirm-submit').click()
    await expect(dialog).toBeHidden()

    // The exact same mutating command a manual scale click would send — nothing invented.
    await expect
      .poll(() => sent(page))
      .toEqual([
        {
          type: 'ScaleDeployment',
          data: { namespace: D.namespace, name: D.name, replicas: 5 },
        },
      ])

    // AgentRecordExecutedAction must not be sent until the mutating command actually resolves.
    expect(await sentTypes(page)).not.toContain('AgentRecordExecutedAction')

    await finish(page, scaleRequestId, true)

    await expect
      .poll(() => sent(page))
      .toContainEqual({
        type: 'AgentRecordExecutedAction',
        data: {
          object: OBJECT,
          suggestion_id: 'sugg-1',
          command_summary: `Scale ${KEY}`,
          outcome: { type: 'Success' },
        },
      })

    // The recorded outcome then reflects back into the card once the backend echoes the
    // updated conversation (as `AgentRecordExecutedAction`'s handler does in the real backend).
    const executed: AgentConversation = {
      ...conversationWith('scale', { replicas: 5 }, 'Reviewed'),
      executed_actions: [
        {
          id: 'exec-1',
          suggested_action_id: 'sugg-1',
          command_summary: `Scale ${KEY}`,
          outcome: { type: 'Success' },
          executed_at: '2026-09-22T00:00:10Z',
        },
      ],
    }
    await emitConversation(page, 11, executed)
    await expect(card.getByTestId('agent-ops-suggested-action-outcome')).toContainText('succeeded')
  })

  test('(d) declining sends only AgentDeclineSuggestedAction, no mutating command', async ({
    page,
  }) => {
    const insp = await selectDeploymentAndOpenAgentTab(page)
    await emitConversation(page, 10, conversationWith('delete'))

    const card = insp.getByTestId('agent-ops-suggested-action-card')
    await clearSent(page)
    await card.getByTestId('agent-ops-suggested-action-decline').click()

    await expect
      .poll(() => sent(page))
      .toEqual([
        { type: 'AgentDeclineSuggestedAction', data: { object: OBJECT, suggestion_id: 'sugg-1' } },
      ])
    const types = await sentTypes(page)
    expect(types).not.toContain('DeleteDeployment')
    expect(types).not.toContain('ScaleDeployment')
    expect(types).not.toContain('RestartDeployment')
    // No confirm dialog was ever shown for a decline.
    await expect(confirmDialog(page)).toBeHidden()
  })

  test('(e) an unmatched action_kind renders informational-only, no Review button', async ({
    page,
  }) => {
    const insp = await selectDeploymentAndOpenAgentTab(page)
    await emitConversation(page, 10, conversationWith('rollback-to-previous-image'))

    const card = insp.getByTestId('agent-ops-suggested-action-card')
    await expect(card).toBeVisible()
    await expect(card.getByTestId('agent-ops-suggested-action-unmatched')).toContainText(
      "isn't directly executable",
    )
    await expect(card.getByTestId('agent-ops-suggested-action-review')).toHaveCount(0)
  })
})
