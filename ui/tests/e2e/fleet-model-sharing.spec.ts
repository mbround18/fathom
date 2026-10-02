/**
 * 046-fleet-model US3 (T038): one definition of "prod", shared — and never at the cost of what
 * the operator already has.
 *
 * Three claims, and each one is a place where the honest rendering and the convenient rendering
 * differ:
 *
 * 1. Publish then adopt reproduces the same group **names** and the same **memberships**. A
 *    teammate who adopts "prod" sees the fleet their colleague sees, not a similarly named shell.
 * 2. A locally overridden label wins, is marked local, and **the shared value it overrode stays
 *    on screen beside it** (FR-013). A UI that silently swallowed the shared value would look
 *    correct and leave the operator unable to see what the team actually agreed.
 * 3. An unreachable store and an upstream removal both leave the adopted copy **in place**, marked
 *    no longer following (FR-014). This spec asserts the adopted copy is still *rendered*: an
 *    implementation that dropped the row on a store error passes every other assertion and looks
 *    fine right up to the moment the operator's work disappears.
 *
 * And T051: `FleetModelLoadFailed` says the file could not be read — with its path and the
 * verbatim reason — and never that the fleet is empty or was reset. The backend leaves the bytes
 * untouched and falls back to defaults (FR-007), so wording that implied a discard would be a lie
 * about the operator's own data (Constitution Principle VI).
 */
import { expect, test, type Locator, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import type {
  AdoptedFleetDefinition,
  FleetGroup,
  FleetMember,
  SharedFleetDefinition,
} from '../../src/protocol/types'
import { clearSent, emit, open, openFleet, sent } from './helpers'

// Radix unmounts a closing dialog only after its exit animation; reduced motion keeps the fleet
// sheet's mount/unmount synchronous on a loaded runner.
test.use({ reducedMotion: 'reduce' })

const PROD_A = 'arn:aws:eks:us-east-1:123456789012:cluster/prod-a'
const PROD_B = 'gke_my-project_us-central1-a_prod-b'

const DEFINITION = 'platform-prod'

/** The store's own words. The UI renders this string and composes nothing of its own. */
const STORE_ERROR = 'open /srv/team/fleet-definitions.json: permission denied (os error 13)'
/** The loader's own words, likewise verbatim. */
const LOAD_ERROR = 'fleet.json: invalid type: string "prod", expected a map at line 7 column 14'
const FLEET_PATH = '/home/op/.fathom/fleet.json'

/** Wording that would tell the operator their data is gone. None of it may appear. */
const LOSS_WORDS = /\b(deleted|removed your|discarded|reset|wiped|lost|empty fleet|no clusters)\b/i

function member(
  context: string,
  labels: Record<string, string> = {},
  groups: string[] = [],
  origins: FleetMember['origins'] = {},
  display_name: string | null = null,
): FleetMember {
  return { context, display_name, labels, origins, availability: 'Present', groups }
}

function group(name: string, selector: string): FleetGroup {
  return { name, selector, origin: 'orgDefault', selector_error: null }
}

function definition(over: Partial<SharedFleetDefinition> = {}): SharedFleetDefinition {
  return {
    id: DEFINITION,
    members: {
      [PROD_A]: { display_name: 'prod-a', labels: { env: 'production', tier: 'gold' } },
      [PROD_B]: { display_name: 'prod-b', labels: { env: 'production', tier: 'silver' } },
    },
    groups: { prod: 'env=production', gold: 'tier=gold' },
    rev: 4,
    shared_by: 'lead@example.com',
    updated_at: '2026-09-30T11:00:00Z',
    removed: false,
    ...over,
  }
}

function adopted(
  def: SharedFleetDefinition = definition(),
  still_shared = true,
): AdoptedFleetDefinition {
  return { definition: def, adopted_at: '2026-10-01T08:00:00Z', still_shared }
}

const row = (page: Page, id: string): Locator =>
  page.locator(`[data-testid="fleet-shared-definition"][data-definition="${id}"]`)

const card = (page: Page, context: string): Locator =>
  page.locator(`[data-testid="fleet-card"][data-context-name="${context}"]`)

const groupMember = (page: Page, name: string, context: string): Locator =>
  page.locator(
    `[data-testid="fleet-group-member"][data-group="${name}"][data-context-name="${context}"]`,
  )

async function openFleetWith(page: Page, contexts: string[] = [PROD_A, PROD_B]): Promise<void> {
  await open(page, 'manual')
  await emit(
    page,
    batch([
      system(1, { type: 'ContextsUpdated', data: { contexts, current: contexts[0] ?? null } }),
    ]),
  )
  await openFleet(page)
}

async function emitModel(
  page: Page,
  seq: number,
  members: FleetMember[],
  groups: FleetGroup[] = [],
): Promise<void> {
  await emit(page, batch([system(seq, { type: 'FleetModelUpdated', data: { members, groups } })]))
}

async function emitShared(
  page: Page,
  seq: number,
  definitions: SharedFleetDefinition[],
  rows: AdoptedFleetDefinition[],
): Promise<void> {
  await emit(
    page,
    batch([
      system(seq, {
        type: 'SharedFleetDefinitionsUpdated',
        data: { definitions, adopted: rows },
      }),
    ]),
  )
}

test('publishing then adopting reproduces the same group names and the same memberships', async ({
  page,
}) => {
  await openFleetWith(page)
  await emitModel(
    page,
    2,
    [
      member(PROD_A, { env: 'production', tier: 'gold' }, ['prod', 'gold']),
      member(PROD_B, { env: 'production', tier: 'silver' }, ['prod']),
    ],
    [group('prod', 'env=production'), group('gold', 'tier=gold')],
  )
  await clearSent(page)

  // The platform lead publishes under an id of their choosing. The UI sends the id; everything
  // about the revision, the attribution and the conflict check is the store's (FR-012).
  const panel = page.getByTestId('fleet-sharing')
  await panel.getByTestId('fleet-share-id-input').fill(DEFINITION)
  await panel.getByTestId('fleet-share-publish').click()
  expect((await sent(page)).filter((c) => c.type === 'PublishFleetDefinition')).toEqual([
    { type: 'PublishFleetDefinition', data: { id: DEFINITION } },
  ])

  // The teammate's side: the definition is in the store, not yet adopted.
  await clearSent(page)
  await emitShared(page, 3, [definition()], [])
  const shared = row(page, DEFINITION)
  await expect(shared).toHaveAttribute('data-adopted', 'no')
  // The same group names, verbatim, with the selector each one actually is.
  await expect(shared.getByTestId('fleet-shared-group')).toHaveCount(2)
  await expect(
    shared.locator('[data-testid="fleet-shared-group"][data-group="prod"]'),
  ).toContainText('env=production')
  await expect(
    shared.locator('[data-testid="fleet-shared-group"][data-group="gold"]'),
  ).toBeVisible()
  await expect(shared.getByTestId('fleet-shared-member')).toHaveCount(2)

  await shared.getByTestId('fleet-definition-adopt').click()
  expect((await sent(page)).filter((c) => c.type === 'AdoptFleetDefinition')).toEqual([
    { type: 'AdoptFleetDefinition', data: { id: DEFINITION } },
  ])

  // What the backend reports after adoption: the same groups, resolved over the same members.
  await emitShared(page, 4, [definition()], [adopted()])
  await emitModel(
    page,
    5,
    [
      member(PROD_A, { env: 'production', tier: 'gold' }, ['prod', 'gold'], {
        'labels/env': 'orgDefault',
        'labels/tier': 'orgDefault',
      }),
      member(PROD_B, { env: 'production', tier: 'silver' }, ['prod'], {
        'labels/env': 'orgDefault',
      }),
    ],
    [group('prod', 'env=production'), group('gold', 'tier=gold')],
  )

  await expect(row(page, DEFINITION)).toHaveAttribute('data-adopted', 'yes')
  await expect(groupMember(page, 'prod', PROD_A)).toBeVisible()
  await expect(groupMember(page, 'prod', PROD_B)).toBeVisible()
  await expect(groupMember(page, 'gold', PROD_A)).toBeVisible()
  await expect(groupMember(page, 'gold', PROD_B)).toHaveCount(0)
})

test('a local label wins, is marked local, and the shared value it overrode stays visible', async ({
  page,
}) => {
  await openFleetWith(page)
  await emitShared(page, 2, [definition()], [adopted()])
  await emitModel(
    page,
    3,
    [
      // The operator edited `env` on prod-a. Everything else still follows the shared copy.
      member(PROD_A, { env: 'canary', tier: 'gold' }, ['gold'], {
        'labels/env': 'local',
        'labels/tier': 'orgDefault',
      }),
      member(PROD_B, { env: 'production', tier: 'silver' }, ['prod'], {
        'labels/env': 'orgDefault',
      }),
    ],
    [group('prod', 'env=production'), group('gold', 'tier=gold')],
  )

  const subject = card(page, PROD_A)
  // The local value is what the fleet uses — the card's persisted row carries it, not the shared one.
  const persisted = subject.getByTestId('fleet-member-labels')
  await expect(persisted.locator('[data-label-key="env"]')).toHaveText('env=canary')

  await subject.getByTestId('fleet-member-edit').click()
  const editor = subject.getByTestId('fleet-member-editor')

  // Marked local...
  const origin = editor.locator('[data-testid="fleet-value-origin"][data-field="labels/env"]')
  await expect(origin).toHaveAttribute('data-origin', 'local')
  await expect(origin).toContainText(/local/i)

  // ...and the shared value it overrode is on screen at the same time (FR-013).
  const overridden = editor.locator(
    '[data-testid="fleet-overridden-value"][data-field="labels/env"]',
  )
  await expect(overridden).toBeVisible()
  await expect(overridden).toContainText('production')
  await expect(
    editor.getByTestId('fleet-editor-label').filter({ hasText: 'env=canary' }),
  ).toHaveCount(1)

  // A value still following the team's definition says so, and claims no override.
  const following = editor.locator('[data-testid="fleet-value-origin"][data-field="labels/tier"]')
  await expect(following).toHaveAttribute('data-origin', 'orgDefault')
  await expect(
    editor.locator('[data-testid="fleet-overridden-value"][data-field="labels/tier"]'),
  ).toHaveCount(0)
})

test('an unreachable store keeps the adopted copy, marked no longer following', async ({
  page,
}) => {
  await openFleetWith(page)
  await emitModel(
    page,
    2,
    [member(PROD_A, { env: 'production' }, ['prod'])],
    [group('prod', 'env=production')],
  )
  await emitShared(page, 3, [definition()], [adopted()])
  await expect(row(page, DEFINITION)).toHaveAttribute('data-following', 'yes')

  await emit(
    page,
    batch([system(4, { type: 'FleetStoreUnavailable', data: { reason: STORE_ERROR } })]),
  )

  // The adopted copy is still here. This is the assertion the story exists for (FR-014).
  const shared = row(page, DEFINITION)
  await expect(shared).toBeVisible()
  await expect(shared).toHaveAttribute('data-adopted', 'yes')
  await expect(shared).toHaveAttribute('data-following', 'no')
  await expect(shared.getByTestId('fleet-shared-group')).toHaveCount(2)
  await expect(shared.getByTestId('fleet-definition-not-following')).toBeVisible()
  // The group it defines still works.
  await expect(groupMember(page, 'prod', PROD_A)).toBeVisible()

  // The banner says the store could not be reached — not that anything was lost.
  const banner = page.getByTestId('fleet-store-unavailable')
  await expect(banner).toBeVisible()
  await expect(banner).not.toHaveText(LOSS_WORDS)
  await expect(banner).toContainText(/still/i)
  // The store's own message, verbatim.
  await expect(banner.getByTestId('fleet-store-unavailable-reason')).toHaveText(STORE_ERROR)
})

test('a definition removed upstream is kept, flagged, and removable only by the operator', async ({
  page,
}) => {
  await openFleetWith(page)
  await emitModel(
    page,
    2,
    [member(PROD_A, { env: 'production' }, ['prod'])],
    [group('prod', 'env=production')],
  )
  await emitShared(page, 3, [definition()], [adopted()])
  await clearSent(page)

  // Removed upstream: the row is kept and `still_shared` is the only thing that changed.
  await emitShared(
    page,
    4,
    [definition({ removed: true })],
    [adopted(definition({ removed: true }), false)],
  )

  const shared = row(page, DEFINITION)
  await expect(shared).toBeVisible()
  await expect(shared).toHaveAttribute('data-adopted', 'yes')
  await expect(shared).toHaveAttribute('data-following', 'no')
  await expect(shared.getByTestId('fleet-shared-member')).toHaveCount(2)
  const flag = shared.getByTestId('fleet-definition-not-following')
  await expect(flag).toContainText(/no longer shared/i)
  await expect(flag).not.toHaveText(LOSS_WORDS)
  await expect(groupMember(page, 'prod', PROD_A)).toBeVisible()

  // It goes away when the operator says so, and not before.
  await shared.getByTestId('fleet-definition-remove').click()
  expect((await sent(page)).filter((c) => c.type === 'RemoveFleetDefinition')).toEqual([
    { type: 'RemoveFleetDefinition', data: { id: DEFINITION } },
  ])
})

test('a fleet file that could not be read says so, with its path and the verbatim reason', async ({
  page,
}) => {
  await openFleetWith(page)
  await emit(
    page,
    batch([
      system(2, { type: 'FleetModelLoadFailed', data: { path: FLEET_PATH, reason: LOAD_ERROR } }),
    ]),
  )

  const failure = page.getByTestId('fleet-load-failure')
  await expect(failure).toBeVisible()
  await expect(failure.getByTestId('fleet-load-failure-path')).toHaveText(FLEET_PATH)
  // The loader's own words, unwrapped and unprefixed.
  await expect(failure.getByTestId('fleet-load-failure-reason')).toHaveText(LOAD_ERROR)
  // The file was left byte-for-byte alone, so nothing here may suggest otherwise (FR-007).
  await expect(failure).not.toHaveText(LOSS_WORDS)
  await expect(failure).toContainText(/could not be read/i)
})

test('with no contexts at all, a load failure is reported instead of an empty fleet', async ({
  page,
}) => {
  await open(page, 'manual')
  await emit(
    page,
    batch([system(1, { type: 'ContextsUpdated', data: { contexts: [], current: null } })]),
  )
  await openFleet(page)
  await emit(
    page,
    batch([
      system(2, { type: 'FleetModelLoadFailed', data: { path: FLEET_PATH, reason: LOAD_ERROR } }),
    ]),
  )

  // "Connect a context to see fleet health" would be a false explanation of an unreadable file.
  const failure = page.getByTestId('fleet-load-failure')
  await expect(failure).toBeVisible()
  await expect(failure.getByTestId('fleet-load-failure-reason')).toHaveText(LOAD_ERROR)
  await expect(failure).not.toHaveText(LOSS_WORDS)
})
