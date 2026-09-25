/**
 * US1 (014-teams-organizations): shared, non-secret connection profiles. Drives the fake
 * backend directly (this suite has no real shared-store server), mirroring
 * `us2-agent-ops-auth.spec.ts`'s pattern: navigate to the "Connection Profiles" section
 * (`team/profiles`, `Team` nav group), assert the load commands it sends on mount, emit
 * `TeamStoreStatusUpdated`/`ConnectionProfilesUpdated` to populate it, then drive
 * publish/adopt/remove and the non-secret-enforcement rejection path.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { emit, finish, navigateTo, nextRequestId, open, sentTypes } from './helpers'

async function goToProfiles(page: Page) {
  await open(page)
  await navigateTo(page, 'Connection Profiles', 'Team')
  await expect
    .poll(() => sentTypes(page))
    .toEqual(expect.arrayContaining(['RefreshTeamStoreStatus', 'RefreshConnectionProfiles']))
}

function panel(page: Page) {
  return page.getByTestId('teams-profiles-panel')
}

test.describe('US1 teams: shared connection profiles', () => {
  test('shows store status and publishes a new profile', async ({ page }) => {
    await goToProfiles(page)

    await emit(
      page,
      batch([
        system(10, {
          type: 'TeamStoreStatusUpdated',
          data: { storePath: '/srv/fathom-team', reachable: true, error: null },
        }),
      ]),
    )
    await expect(panel(page).getByTestId('teams-store-status')).toHaveAttribute(
      'data-reachable',
      'true',
    )
    await expect(panel(page).getByTestId('teams-store-status')).toContainText('/srv/fathom-team')

    await panel(page).getByTestId('teams-publish-context-alias').fill('prod')
    await panel(page)
      .getByTestId('teams-publish-api-server')
      .fill('https://cluster.example.com:6443')
    await panel(page).getByTestId('teams-publish-default-namespace').fill('default')

    const requestId = await nextRequestId(page)
    await panel(page).getByTestId('teams-publish-profile').click()
    await expect.poll(() => sentTypes(page)).toContain('PublishConnectionProfile')
    const cmds = await page.evaluate(() => window.__fathomFake?.sent() ?? [])
    const publishCmd = cmds.find((c) => c.type === 'PublishConnectionProfile')
    expect(publishCmd).toEqual({
      type: 'PublishConnectionProfile',
      data: {
        context_alias: 'prod',
        api_server: 'https://cluster.example.com:6443',
        default_namespace: 'default',
      },
    })
    await finish(page, requestId, true)

    await emit(
      page,
      batch([
        system(11, {
          type: 'ConnectionProfilesUpdated',
          data: {
            adopted: [],
            available: [
              {
                id: 'p1',
                contextAlias: 'prod',
                apiServer: 'https://cluster.example.com:6443',
                defaultNamespace: 'default',
                rev: 1,
                sharedBy: 'alice',
                updatedAt: '2025-01-01T00:00:00Z',
                removed: false,
              },
            ],
          },
        }),
      ]),
    )
    await expect(panel(page).getByTestId('teams-available-row')).toContainText('prod')
    await expect(panel(page).getByTestId('teams-available-row')).toContainText('alice')

    // Adopt it.
    await panel(page).getByTestId('teams-adopt-profile').click()
    await expect.poll(() => sentTypes(page)).toContain('AdoptConnectionProfile')
    const adoptCmd = (await page.evaluate(() => window.__fathomFake?.sent() ?? [])).find(
      (c) => c.type === 'AdoptConnectionProfile',
    )
    expect(adoptCmd).toEqual({ type: 'AdoptConnectionProfile', data: { profile_id: 'p1' } })

    await emit(
      page,
      batch([
        system(12, {
          type: 'ConnectionProfilesUpdated',
          data: {
            adopted: [
              {
                profile: {
                  id: 'p1',
                  contextAlias: 'prod',
                  apiServer: 'https://cluster.example.com:6443',
                  defaultNamespace: 'default',
                  rev: 1,
                  sharedBy: 'alice',
                  updatedAt: '2025-01-01T00:00:00Z',
                  removed: false,
                },
                adoptedAt: '2025-01-02T00:00:00Z',
                stillShared: true,
              },
            ],
            available: [],
          },
        }),
      ]),
    )
    await expect(panel(page).getByTestId('teams-adopted-row')).toContainText('prod')
    await expect(panel(page).getByTestId('teams-profile-no-longer-shared')).toHaveCount(0)
  })

  test('a publish that looks like it carries a credential is rejected, never reaching the store as-is', async ({
    page,
  }) => {
    await goToProfiles(page)

    await panel(page).getByTestId('teams-publish-context-alias').fill('prod')
    // A pasted Bearer token in the API-server field — the non-secret guarantee's backstop.
    await panel(page)
      .getByTestId('teams-publish-api-server')
      .fill('Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abc123')

    const requestId = await nextRequestId(page)
    await panel(page).getByTestId('teams-publish-profile').click()
    await expect.poll(() => sentTypes(page)).toContain('PublishConnectionProfile')
    await finish(page, requestId, true)

    await emit(
      page,
      batch([
        system(20, {
          type: 'TeamPublishRejected',
          data: {
            rejection: {
              kind: 'suspectedSecret',
              field: 'api_server',
              reason: 'looks like it contains credential/secret material (matched "bearer ")',
            },
          },
        }),
      ]),
    )
    await expect(panel(page).getByTestId('teams-publish-rejection')).toContainText(
      'Publish refused',
    )
    await expect(panel(page).getByTestId('teams-publish-rejection')).toContainText('api_server')
  })

  test('a profile removed upstream is shown as no longer shared, not silently dropped', async ({
    page,
  }) => {
    await goToProfiles(page)

    await emit(
      page,
      batch([
        system(30, {
          type: 'ConnectionProfilesUpdated',
          data: {
            adopted: [
              {
                profile: {
                  id: 'p2',
                  contextAlias: 'staging',
                  apiServer: 'https://staging.example.com:6443',
                  defaultNamespace: null,
                  rev: 2,
                  sharedBy: 'bob',
                  updatedAt: '2025-01-03T00:00:00Z',
                  removed: true,
                },
                adoptedAt: '2025-01-01T00:00:00Z',
                stillShared: false,
              },
            ],
            available: [],
          },
        }),
      ]),
    )
    await expect(panel(page).getByTestId('teams-adopted-row')).toContainText('staging')
    await expect(panel(page).getByTestId('teams-profile-no-longer-shared')).toBeVisible()
  })
})
