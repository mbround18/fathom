/**
 * 046-fleet-model US1 (T025): an operator names and labels the clusters they actually have.
 *
 * SC-001: the cluster is identified without reading the context string, and the context string is
 * still there, whole and copyable (FR-002). An invalid label is refused in the backend's own words
 * and nothing is shown as persisted (FR-003). An `Absent` member keeps every byte of its metadata
 * (FR-009). The UI holds no copy of the label grammar (Principle II), so the only rejection text
 * this spec accepts is the one the backend emitted.
 */
import { expect, test, type Locator, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import type { FleetMember } from '../../src/protocol/types'
import { clearSent, emit, open, openFleet, prepareClipboard, readClipboard, sent } from './helpers'

// Radix unmounts a closing dialog only after its exit animation; reduced motion keeps the fleet
// sheet's mount/unmount synchronous on a loaded runner (see us1-fleet-dashboard.spec.ts).
test.use({ reducedMotion: 'reduce' })

/** What `aws eks update-kubeconfig` writes, and the reason a display name exists at all. */
const ARN = 'arn:aws:eks:us-east-1:123456789012:cluster/prod-blue'
const RETIRED = 'gke_my-project_us-central1-a_retired'

/** The backend's own words. The UI must render this string and never compose its own. */
const REJECTION = 'label key "env/team/prod" is invalid: a key may contain at most one "/"'
const REJECTION_FIELD = 'labels/env/team/prod'

function member(context: string, over: Partial<FleetMember> = {}): FleetMember {
  return {
    context,
    display_name: null,
    labels: {},
    origins: {},
    availability: 'Present',
    ...over,
  }
}

const PROD_BLUE = member(ARN, {
  display_name: 'prod-blue',
  labels: { env: 'prod', region: 'us-east-1' },
  origins: { display_name: 'local', 'labels/env': 'local', 'labels/region': 'local' },
})

const card = (page: Page, context: string): Locator =>
  page.locator(`[data-testid="fleet-card"][data-context-name="${context}"]`)

/** Open the shell with both contexts in the kubeconfig and the fleet sheet showing. */
async function openFleetWith(page: Page, contexts: string[] = [ARN, RETIRED]): Promise<void> {
  await open(page, 'manual')
  await emit(
    page,
    batch([
      system(1, { type: 'ContextsUpdated', data: { contexts, current: contexts[0] ?? null } }),
    ]),
  )
  await openFleet(page)
}

/** What the backend emits after reading `~/.fathom/fleet.json`. */
async function emitModel(page: Page, seq: number, members: FleetMember[]): Promise<void> {
  await emit(
    page,
    batch([system(seq, { type: 'FleetModelUpdated', data: { members, groups: [] } })]),
  )
}

test('the card is headed by its display name and still carries the whole context string', async ({
  page,
}) => {
  await openFleetWith(page)
  await emitModel(page, 2, [PROD_BLUE])

  const subject = card(page, ARN)
  // Both, never one instead of the other (FR-002).
  await expect(subject.getByTestId('fleet-display-name')).toHaveText('prod-blue')
  await expect(subject.getByTestId('fleet-context-string')).toHaveText(ARN)
  await expect(subject.getByTestId('fleet-display-name')).toBeVisible()
  await expect(subject.getByTestId('fleet-context-string')).toBeVisible()

  // The labels this story writes, which every later story reads.
  const labels = subject.getByTestId('fleet-member-labels')
  await expect(labels.getByTestId('fleet-member-label')).toHaveCount(2)
  await expect(labels.locator('[data-label-key="env"]')).toHaveText('env=prod')
  await expect(labels.locator('[data-label-key="region"]')).toHaveText('region=us-east-1')

  // A context with no fleet record is still identified by its context string alone.
  await expect(card(page, RETIRED).getByTestId('fleet-context-string')).toHaveText(RETIRED)
  await expect(card(page, RETIRED).getByTestId('fleet-display-name')).toHaveCount(0)
})

test('the context string copies verbatim, not truncated or prettified', async ({
  page,
  context,
  browserName,
}) => {
  await prepareClipboard(page, context, browserName)
  await openFleetWith(page)
  await emitModel(page, 2, [PROD_BLUE])

  const subject = card(page, ARN)
  // The rendered text is the context name itself — truncation is CSS, never the string.
  await expect(subject.getByTestId('fleet-context-string')).toHaveText(ARN)
  await subject.getByTestId('fleet-context-copy').click()
  await expect.poll(async () => await readClipboard(page, browserName)).toBe(ARN)
})

test('an invalid label is refused in the backend’s words and nothing is shown as persisted', async ({
  page,
}) => {
  await openFleetWith(page)
  await emitModel(page, 2, [PROD_BLUE])
  await clearSent(page)

  const subject = card(page, ARN)
  await subject.getByTestId('fleet-member-edit').click()
  const editor = subject.getByTestId('fleet-member-editor')
  await editor.getByTestId('fleet-label-key-input').fill('env/team/prod')
  await editor.getByTestId('fleet-label-value-input').fill('yes')
  await editor.getByTestId('fleet-label-add').click()
  await editor.getByTestId('fleet-labels-save').click()

  // The UI implements no grammar, so it sends what was typed and lets the backend judge it.
  const labelCmds = (await sent(page)).filter((c) => c.type === 'SetFleetMemberLabels')
  expect(labelCmds).toHaveLength(1)
  expect(labelCmds[0]).toEqual({
    type: 'SetFleetMemberLabels',
    data: { context: ARN, labels: { env: 'prod', region: 'us-east-1', 'env/team/prod': 'yes' } },
  })

  await emit(
    page,
    batch([
      system(3, {
        type: 'FleetModelRejected',
        data: { reason: REJECTION, field: REJECTION_FIELD },
      }),
    ]),
  )

  const rejection = subject.getByTestId('fleet-member-rejection')
  await expect(rejection).toHaveText(REJECTION)
  await expect(rejection).toHaveAttribute('data-field', REJECTION_FIELD)

  // Nothing is shown as persisted: the card's label row still carries exactly the two labels the
  // model reported. (The editor keeps the refused draft on screen so it can be corrected.)
  const persisted = subject.getByTestId('fleet-member-labels')
  await expect(persisted.getByTestId('fleet-member-label')).toHaveCount(2)
  await expect(persisted.locator('[data-label-key="env/team/prod"]')).toHaveCount(0)
})

test('a display name and labels are both there after a reload', async ({ page }) => {
  await openFleetWith(page)
  await emitModel(page, 2, [member(ARN)])
  await clearSent(page)

  const subject = card(page, ARN)
  await subject.getByTestId('fleet-member-edit').click()
  const editor = subject.getByTestId('fleet-member-editor')
  await editor.getByTestId('fleet-display-name-input').fill('prod-blue')
  await editor.getByTestId('fleet-display-name-save').click()
  await editor.getByTestId('fleet-label-key-input').fill('env')
  await editor.getByTestId('fleet-label-value-input').fill('prod')
  await editor.getByTestId('fleet-label-add').click()
  await editor.getByTestId('fleet-labels-save').click()

  // Trim-to-None is the backend's rule: the UI sends what the operator typed.
  expect(await sent(page)).toEqual(
    expect.arrayContaining([
      { type: 'SetFleetMemberDisplayName', data: { context: ARN, display_name: 'prod-blue' } },
      { type: 'SetFleetMemberLabels', data: { context: ARN, labels: { env: 'prod' } } },
    ]),
  )

  // Reload. The backend reads `fleet.json` at launch and reports the member again.
  await openFleetWith(page)
  await emitModel(page, 2, [
    member(ARN, { display_name: 'prod-blue', labels: { env: 'prod' }, origins: {} }),
  ])
  const reloaded = card(page, ARN)
  await expect(reloaded.getByTestId('fleet-display-name')).toHaveText('prod-blue')
  await expect(
    reloaded.getByTestId('fleet-member-labels').locator('[data-label-key="env"]'),
  ).toHaveText('env=prod')
  await expect(reloaded.getByTestId('fleet-context-string')).toHaveText(ARN)
})

test('an absent member reads as unavailable with its metadata intact', async ({ page }) => {
  await openFleetWith(page)
  await emitModel(page, 2, [
    PROD_BLUE,
    member(RETIRED, {
      display_name: 'retired-central',
      labels: { env: 'dev', owner: 'payments' },
      availability: 'Absent',
    }),
  ])

  const gone = card(page, RETIRED)
  await expect(gone).toHaveAttribute('data-availability', 'Absent')
  await expect(gone.getByTestId('fleet-availability')).toHaveText('Unavailable')
  // Metadata intact — nothing about it was erased by the context leaving the kubeconfig (FR-009).
  await expect(gone.getByTestId('fleet-display-name')).toHaveText('retired-central')
  await expect(gone.getByTestId('fleet-context-string')).toHaveText(RETIRED)
  const kept = gone.getByTestId('fleet-member-labels')
  await expect(kept.getByTestId('fleet-member-label')).toHaveCount(2)
  await expect(kept.locator('[data-label-key="owner"]')).toHaveText('owner=payments')

  // A present member is not marked unavailable.
  await expect(card(page, ARN)).toHaveAttribute('data-availability', 'Present')
  await expect(card(page, ARN).getByTestId('fleet-availability')).toHaveCount(0)
})
