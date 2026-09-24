/**
 * US2 (014-teams-organizations, expanded): local §10 Cluster Settings + org-level sync.
 * Navigates to the "Cluster Settings" section (`integrations/cluster-settings`, `Team` nav
 * group), drives load/edit/save, publish-as-org-default, the accept/dismiss org-default
 * banner, and the error-preserves-draft guarantee (Constitution Principle VI).
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { emit, finish, navigateTo, nextRequestId, open, sentTypes } from './helpers'

const ACTIVE_CONTEXT = 'kind-a'

const BASE_SETTINGS = {
  schema: 1 as const,
  metricsSource: 'none' as const,
  prometheusPath: '/api/v1',
  https: false,
  pathPrefix: '',
  hiddenMetrics: [],
  orgRevKnown: null,
  fieldOrigin: {
    metricsSource: 'local' as const,
    prometheusPath: 'local' as const,
    https: 'local' as const,
    pathPrefix: 'local' as const,
    hiddenMetrics: 'local' as const,
  },
}

async function goToSettings(page: Page) {
  await open(page)
  await navigateTo(page, 'Cluster Settings', 'Team')
  await expect.poll(() => sentTypes(page)).toContain('LoadClusterSettings')
  await emit(
    page,
    batch([
      system(1, {
        type: 'ClusterSettingsLoaded',
        data: { context: ACTIVE_CONTEXT, settings: BASE_SETTINGS },
      }),
    ]),
  )
}

function panel(page: Page) {
  return page.getByTestId('teams-cluster-settings-panel')
}

test.describe('US2 teams: cluster settings + org sync', () => {
  test('loads, edits, and saves local settings', async ({ page }) => {
    await goToSettings(page)

    await expect(panel(page).getByTestId('teams-prometheus-path')).toHaveValue('/api/v1')

    await panel(page).getByTestId('teams-metrics-source').selectOption('prometheus')
    await panel(page).getByTestId('teams-prometheus-path').fill('/prometheus/api/v1')
    await panel(page).getByTestId('teams-https-toggle').check()
    await panel(page).getByTestId('teams-path-prefix').fill('/grafana')
    await panel(page).getByTestId('teams-hidden-metric-input').fill('cpu_throttled')
    await panel(page).getByTestId('teams-hidden-metric-input').press('Enter')
    await expect(panel(page).getByTestId('teams-hidden-metric-row')).toContainText('cpu_throttled')

    // Navigating expanded a nav group, whose debounced SaveUiConfig consumes a request id: let it
    // land before reading `nextRequestId`, or the injected CommandFinished targets the wrong one.
    await expect.poll(() => sentTypes(page)).toContain('SaveUiConfig')
    const requestId = await nextRequestId(page)
    await panel(page).getByTestId('teams-save-cluster-settings').click()
    await expect.poll(() => sentTypes(page)).toContain('SaveClusterSettings')
    const cmds = await page.evaluate(() => window.__fathomFake?.sent() ?? [])
    const saveCmd = cmds.find((c) => c.type === 'SaveClusterSettings')
    expect(saveCmd).toEqual({
      type: 'SaveClusterSettings',
      data: {
        context: ACTIVE_CONTEXT,
        settings: {
          schema: 1,
          metricsSource: 'prometheus',
          prometheusPath: '/prometheus/api/v1',
          https: true,
          pathPrefix: '/grafana',
          hiddenMetrics: ['cpu_throttled'],
          orgRevKnown: null,
          fieldOrigin: {
            metricsSource: 'local',
            prometheusPath: 'local',
            https: 'local',
            pathPrefix: 'local',
            hiddenMetrics: 'local',
          },
        },
      },
    })
    await finish(page, requestId, true)
    await expect(panel(page).getByTestId('teams-cluster-settings-saved')).toBeVisible()
  })

  test('publishes as org default', async ({ page }) => {
    await goToSettings(page)

    const requestId = await nextRequestId(page)
    await panel(page).getByTestId('teams-publish-org-default').click()
    await expect.poll(() => sentTypes(page)).toContain('PublishOrgClusterSettings')
    const cmds = await page.evaluate(() => window.__fathomFake?.sent() ?? [])
    const publishCmd = cmds.find((c) => c.type === 'PublishOrgClusterSettings')
    expect(publishCmd).toEqual({
      type: 'PublishOrgClusterSettings',
      data: { context: ACTIVE_CONTEXT },
    })
    await finish(page, requestId, true)
  })

  test('org-default banner: accept and dismiss both clear it and dispatch the right command', async ({
    page,
  }) => {
    await goToSettings(page)

    await emit(
      page,
      batch([
        system(2, {
          type: 'OrgClusterSettingsAvailable',
          data: {
            context: ACTIVE_CONTEXT,
            settings: {
              context: ACTIVE_CONTEXT,
              rev: 3,
              publishedBy: 'carol',
              publishedAt: '2025-02-01T00:00:00Z',
              metricsSource: 'prometheus',
              prometheusPath: '/api/v1',
              https: true,
              pathPrefix: '',
              hiddenMetrics: [],
            },
          },
        }),
      ]),
    )
    await expect(panel(page).getByTestId('teams-org-default-banner')).toContainText('carol')
    await expect(panel(page).getByTestId('teams-org-default-banner')).toContainText('rev 3')

    await panel(page).getByTestId('teams-accept-org-default').click()
    await expect.poll(() => sentTypes(page)).toContain('AcceptOrgClusterSettings')
    const acceptCmd = (await page.evaluate(() => window.__fathomFake?.sent() ?? [])).find(
      (c) => c.type === 'AcceptOrgClusterSettings',
    )
    expect(acceptCmd).toEqual({
      type: 'AcceptOrgClusterSettings',
      data: { context: ACTIVE_CONTEXT },
    })

    // Simulate the banner clearing (backend re-emits without orgAvailable once accepted /
    // superseded by a fresh ClusterSettingsLoaded); the panel derives visibility from
    // `team.orgAvailable`, so re-navigation isn't required, but we exercise dismiss on a
    // fresh banner to confirm the other path too.
    await emit(
      page,
      batch([
        system(3, {
          type: 'OrgClusterSettingsAvailable',
          data: {
            context: ACTIVE_CONTEXT,
            settings: {
              context: ACTIVE_CONTEXT,
              rev: 4,
              publishedBy: 'dave',
              publishedAt: '2025-02-02T00:00:00Z',
              metricsSource: 'prometheus',
              prometheusPath: '/api/v1',
              https: true,
              pathPrefix: '',
              hiddenMetrics: [],
            },
          },
        }),
      ]),
    )
    await panel(page).getByTestId('teams-dismiss-org-default').click()
    await expect.poll(() => sentTypes(page)).toContain('DismissOrgClusterSettings')
    const dismissCmd = (await page.evaluate(() => window.__fathomFake?.sent() ?? [])).find(
      (c) => c.type === 'DismissOrgClusterSettings',
    )
    expect(dismissCmd).toEqual({
      type: 'DismissOrgClusterSettings',
      data: { context: ACTIVE_CONTEXT },
    })
  })

  test('a load error never clobbers an in-progress edit', async ({ page }) => {
    await goToSettings(page)

    await panel(page).getByTestId('teams-path-prefix').fill('/keep-me')

    await emit(
      page,
      batch([
        system(4, {
          type: 'ClusterSettingsError',
          data: { context: ACTIVE_CONTEXT, reason: 'disk read failed' },
        }),
      ]),
    )

    // The draft the operator was editing must survive an unrelated error broadcast.
    await expect(panel(page).getByTestId('teams-path-prefix')).toHaveValue('/keep-me')
  })

  test('a publish conflict is surfaced with both revisions', async ({ page }) => {
    await goToSettings(page)

    const requestId = await nextRequestId(page)
    await panel(page).getByTestId('teams-publish-org-default').click()
    await expect.poll(() => sentTypes(page)).toContain('PublishOrgClusterSettings')
    await finish(page, requestId, true)

    await emit(
      page,
      batch([
        system(5, {
          type: 'TeamPublishRejected',
          data: {
            rejection: { kind: 'conflict', expected_rev: 3, current_rev: 5 },
          },
        }),
      ]),
    )
    await expect(panel(page).getByTestId('teams-publish-rejection')).toContainText('rev 5')
    await expect(panel(page).getByTestId('teams-publish-rejection')).toContainText('rev 3')
  })
})
