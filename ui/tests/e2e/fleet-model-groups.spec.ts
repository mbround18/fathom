/**
 * 046-fleet-model US2 (T030): groups are selectors, and a rollup cannot hide what it missed.
 *
 * A group is one expression (SC-002): there is no per-cluster filing step, and a cluster that
 * gains a matching label joins every matching group with no further action. A group matching
 * nothing says so and is neither hidden nor removed (FR-005). Members matching no group stay
 * reachable through an explicit ungrouped listing (FR-017).
 *
 * The assertion this story exists for: a rollup with `members_covered < members_total` states
 * `n of m`, **names** the member it could not read, and says *why* — and `notPermitted`,
 * `unreachable` and `absent` are three distinct renderings (FR-010, FR-011, SC-004). The product
 * defect this prevents is `ui/src/resources/nav.ts:64-69`, where a *denied* CRD scan renders as
 * "not installed": a sum over the clusters it could read, presented as the whole.
 *
 * The UI holds no copy of the selector grammar (Principle II): the only rejection text this spec
 * accepts for an invalid selector is the string the backend emitted.
 */
import { expect, test, type Locator, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import type { FleetGroup, FleetGroupRollup, FleetMember } from '../../src/protocol/types'
import { clearSent, emit, open, openFleet, sent } from './helpers'

// Radix unmounts a closing dialog only after its exit animation; reduced motion keeps the fleet
// sheet's mount/unmount synchronous on a loaded runner (see us1-fleet-dashboard.spec.ts).
test.use({ reducedMotion: 'reduce' })

const PROD_A = 'arn:aws:eks:us-east-1:123456789012:cluster/prod-a'
const PROD_B = 'gke_my-project_us-central1-a_prod-b'
const DEV = 'kind-dev'
const LOOSE = 'kind-scratch'

/** The backend's own parse error. The UI renders this string and never composes its own. */
const PARSE_ERROR = 'invalid selector "env = = prod": expected a value after "=" at offset 6'

/**
 * A member as `FleetModelUpdated` carries it: `groups` is the membership the backend resolved
 * (FR-004). The UI parses no selector, so the fixture states the answer the backend would give.
 */
function member(
  context: string,
  labels: Record<string, string> = {},
  groups: string[] = [],
): FleetMember {
  return { context, display_name: null, labels, origins: {}, availability: 'Present', groups }
}

function group(name: string, selector: string, selector_error: string | null = null): FleetGroup {
  return { name, selector, origin: 'local', selector_error }
}

function rollup(over: Partial<FleetGroupRollup> & { group: string }): FleetGroupRollup {
  return {
    members_total: 1,
    members_covered: 1,
    gaps: [],
    node_ready: 0,
    node_total: 0,
    unhealthy_pods: 0,
    ...over,
  }
}

const groupSection = (page: Page, name: string): Locator =>
  page.locator(`[data-testid="fleet-group"][data-group="${name}"]`)

const groupMember = (page: Page, name: string, context: string): Locator =>
  page.locator(
    `[data-testid="fleet-group-member"][data-group="${name}"][data-context-name="${context}"]`,
  )

const gap = (page: Page, name: string, context: string): Locator =>
  page.locator(`[data-testid="fleet-gap"][data-group="${name}"][data-context-name="${context}"]`)

/** Open the shell with the kubeconfig's contexts and the fleet sheet showing. */
async function openFleetWith(page: Page, contexts: string[]): Promise<void> {
  await open(page, 'manual')
  await emit(
    page,
    batch([
      system(1, { type: 'ContextsUpdated', data: { contexts, current: contexts[0] ?? null } }),
    ]),
  )
  await openFleet(page)
}

/** What the backend emits after reading `~/.fathom/fleet.json` and resolving availability. */
async function emitModel(
  page: Page,
  seq: number,
  members: FleetMember[],
  groups: FleetGroup[],
): Promise<void> {
  await emit(page, batch([system(seq, { type: 'FleetModelUpdated', data: { members, groups } })]))
}

test('one selector expression lists the group, with no per-cluster filing step', async ({
  page,
}) => {
  await openFleetWith(page, [PROD_A, PROD_B, DEV])
  await clearSent(page)
  await emitModel(
    page,
    2,
    [
      member(PROD_A, { env: 'prod', region: 'us-east-1' }, ['prod', 'us']),
      member(PROD_B, { env: 'prod', region: 'us-central1' }, ['prod']),
      member(DEV, { env: 'dev' }),
    ],
    [group('prod', 'env=prod'), group('us', 'region in (us-east-1, us-west-2)')],
  )

  // Exactly the matching members, under every group they match (FR-004).
  await expect(groupMember(page, 'prod', PROD_A)).toBeVisible()
  await expect(groupMember(page, 'prod', PROD_B)).toBeVisible()
  await expect(groupMember(page, 'prod', DEV)).toHaveCount(0)
  await expect(groupMember(page, 'us', PROD_A)).toBeVisible()
  await expect(groupMember(page, 'us', PROD_B)).toHaveCount(0)
  await expect(groupSection(page, 'prod').getByTestId('fleet-group-member')).toHaveCount(2)

  // The group's own expression is on screen, verbatim as typed.
  await expect(groupSection(page, 'prod').getByTestId('fleet-group-selector')).toHaveText(
    'env=prod',
  )

  // SC-002: nothing was filed per cluster. The view asks for each group's rollup and nothing else.
  const commands = await sent(page)
  const rollups = commands.filter((c) => c.type === 'RequestFleetGroupRollup')
  expect(new Set(rollups.map((c) => c.data.group))).toEqual(new Set(['prod', 'us']))
  const types = commands.map((c) => c.type)
  expect(types).not.toContain('SetFleetMemberLabels')
  expect(types).not.toContain('SaveFleetGroup')
})

test('labelling a cluster joins it to every matching group with no further action', async ({
  page,
}) => {
  await openFleetWith(page, [PROD_A, DEV])
  await emitModel(
    page,
    2,
    [member(PROD_A, { env: 'prod' }, ['prod']), member(DEV, { env: 'dev' })],
    [group('prod', 'env=prod')],
  )
  await expect(groupMember(page, 'prod', DEV)).toHaveCount(0)
  await clearSent(page)

  // The operator labels `kind-dev` env=prod elsewhere; the model update is the only input.
  await emitModel(
    page,
    3,
    [member(PROD_A, { env: 'prod' }, ['prod']), member(DEV, { env: 'prod' }, ['prod'])],
    [group('prod', 'env=prod')],
  )

  await expect(groupMember(page, 'prod', DEV)).toBeVisible()
  await expect(groupSection(page, 'prod').getByTestId('fleet-group-member')).toHaveCount(2)
  // No group was re-saved and no member was filed: membership followed the label (SC-002).
  const types = (await sent(page)).map((c) => c.type)
  expect(types).not.toContain('SaveFleetGroup')
  expect(types).not.toContain('SetFleetMemberLabels')
})

test('a group matching nothing says so, and is neither hidden nor removed', async ({ page }) => {
  await openFleetWith(page, [DEV])
  await clearSent(page)
  await emitModel(page, 2, [member(DEV, { env: 'dev' })], [group('staging', 'env=staging')])

  const staging = groupSection(page, 'staging')
  await expect(staging).toBeVisible()
  await expect(staging.getByTestId('fleet-group-empty')).toHaveText(/matches no cluster/i)
  await expect(staging.getByTestId('fleet-group-member')).toHaveCount(0)
  // FR-005: shown as matching nothing, not deleted behind the operator's back.
  expect((await sent(page)).map((c) => c.type)).not.toContain('DeleteFleetGroup')
})

test('members matching no group are reachable through an explicit ungrouped listing', async ({
  page,
}) => {
  await openFleetWith(page, [PROD_A, LOOSE])
  await emitModel(
    page,
    2,
    [member(PROD_A, { env: 'prod' }, ['prod']), member(LOOSE)],
    [group('prod', 'env=prod')],
  )

  const ungrouped = page.getByTestId('fleet-ungrouped')
  await expect(ungrouped).toBeVisible()
  await expect(
    ungrouped.locator(`[data-testid="fleet-ungrouped-member"][data-context-name="${LOOSE}"]`),
  ).toBeVisible()
  await expect(
    ungrouped.locator(`[data-testid="fleet-ungrouped-member"][data-context-name="${PROD_A}"]`),
  ).toHaveCount(0)
})

test('a partial rollup states n of m, names the member it missed, and says which kind of blind spot it was', async ({
  page,
}) => {
  await openFleetWith(page, [PROD_A, PROD_B, DEV, LOOSE])
  await emitModel(
    page,
    2,
    [
      member(PROD_A, { env: 'prod' }, ['prod']),
      member(PROD_B, { env: 'prod' }, ['prod']),
      member(DEV, { env: 'prod' }, ['prod']),
      member(LOOSE, { env: 'prod' }, ['prod']),
    ],
    [group('prod', 'env=prod')],
  )

  await emit(
    page,
    batch([
      system(3, {
        type: 'FleetGroupRollupUpdated',
        data: rollup({
          group: 'prod',
          members_total: 4,
          members_covered: 1,
          node_ready: 3,
          node_total: 4,
          unhealthy_pods: 2,
          gaps: [
            {
              context: PROD_B,
              reason: {
                kind: 'notPermitted',
                detail: 'nodes is forbidden: User "deploy" cannot list resource "nodes"',
              },
            },
            {
              context: DEV,
              reason: { kind: 'unreachable', detail: 'tcp connect timed out after 5s' },
            },
            { context: LOOSE, reason: { kind: 'absent' } },
          ],
        }),
      }),
    ]),
  )

  const prod = groupSection(page, 'prod')
  // The sum never appears without the coverage beside it (FR-010, SC-004).
  const coverage = prod.getByTestId('fleet-group-coverage')
  await expect(coverage).toContainText('1 of 4 clusters')
  await expect(coverage).toContainText('3 of 4 nodes ready')
  await expect(prod).toHaveAttribute('data-coverage', 'partial')

  // Each missed member is named, not counted (FR-010).
  const denied = gap(page, 'prod', PROD_B)
  const unreachable = gap(page, 'prod', DEV)
  const absent = gap(page, 'prod', LOOSE)
  for (const g of [denied, unreachable, absent]) await expect(g).toBeVisible()
  await expect(denied).toContainText(PROD_B)
  await expect(denied).toContainText(/not permitted/i)
  await expect(denied).toContainText(
    'nodes is forbidden: User "deploy" cannot list resource "nodes"',
  )

  // FR-011: three reasons, three renderings. A UI that collapsed them would pass a test that only
  // looked at `notPermitted`, so compare them against each other.
  await expect(denied).toHaveAttribute('data-reason', 'notPermitted')
  await expect(unreachable).toHaveAttribute('data-reason', 'unreachable')
  await expect(absent).toHaveAttribute('data-reason', 'absent')
  await expect(unreachable).toContainText(/unreachable/i)
  await expect(unreachable).toContainText('tcp connect timed out after 5s')
  await expect(absent).toContainText(/absent from the kubeconfig/i)

  const texts = await Promise.all(
    [denied, unreachable, absent].map(async (g) => ((await g.textContent()) ?? '').trim()),
  )
  expect(new Set(texts).size).toBe(3)
  const classes = await Promise.all(
    [denied, unreachable, absent].map((g) =>
      g.getByTestId('fleet-gap-reason').getAttribute('class'),
    ),
  )
  expect(new Set(classes).size).toBe(3)
})

test('a fully covered rollup still states its coverage', async ({ page }) => {
  await openFleetWith(page, [PROD_A, PROD_B])
  await emitModel(
    page,
    2,
    [member(PROD_A, { env: 'prod' }, ['prod']), member(PROD_B, { env: 'prod' }, ['prod'])],
    [group('prod', 'env=prod')],
  )
  await emit(
    page,
    batch([
      system(3, {
        type: 'FleetGroupRollupUpdated',
        data: rollup({
          group: 'prod',
          members_total: 2,
          members_covered: 2,
          node_ready: 9,
          node_total: 9,
          unhealthy_pods: 0,
        }),
      }),
    ]),
  )

  const prod = groupSection(page, 'prod')
  await expect(prod).toHaveAttribute('data-coverage', 'complete')
  await expect(prod.getByTestId('fleet-group-coverage')).toContainText('2 of 2 clusters')
  await expect(prod.getByTestId('fleet-gap')).toHaveCount(0)
})

test('the filter states how many of how many members it is showing', async ({ page }) => {
  await openFleetWith(page, [PROD_A, PROD_B, DEV])
  await emitModel(
    page,
    2,
    [
      member(PROD_A, { env: 'prod' }, ['prod']),
      member(PROD_B, { env: 'prod' }, ['prod']),
      member(DEV, { env: 'dev' }),
    ],
    [group('prod', 'env=prod')],
  )
  await expect(page.getByTestId('fleet-filter-count')).toHaveText('Showing 3 of 3 clusters')

  await page.getByTestId('fleet-filter').fill('env=prod')
  await expect(page.getByTestId('fleet-filter-count')).toHaveText('Showing 2 of 3 clusters')
  await expect(groupMember(page, 'prod', PROD_A)).toBeVisible()
  await expect(page.getByTestId('fleet-ungrouped')).toHaveCount(0)
})

test('an invalid selector is refused in the backend words, and creates no group', async ({
  page,
}) => {
  await openFleetWith(page, [PROD_A])
  await emitModel(page, 2, [member(PROD_A, { env: 'prod' })], [])
  await clearSent(page)

  await page.getByTestId('fleet-group-name-input').fill('prod')
  await page.getByTestId('fleet-group-selector-input').fill('env = = prod')
  await page.getByTestId('fleet-group-save').click()

  // The backend owns the grammar: the UI sends the expression verbatim and waits (Principle II).
  const saves = (await sent(page)).filter((c) => c.type === 'SaveFleetGroup')
  expect(saves).toEqual([
    { type: 'SaveFleetGroup', data: { name: 'prod', selector: 'env = = prod' } },
  ])

  await emit(
    page,
    batch([
      system(3, {
        type: 'FleetModelRejected',
        data: { reason: PARSE_ERROR, field: 'selector' },
      }),
    ]),
  )

  await expect(page.getByTestId('fleet-group-error')).toHaveText(PARSE_ERROR)
  await expect(groupSection(page, 'prod')).toHaveCount(0)
  await expect(page.getByTestId('fleet-group')).toHaveCount(0)
})
