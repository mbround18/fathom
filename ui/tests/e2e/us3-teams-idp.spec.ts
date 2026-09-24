/**
 * 033 US3: Teams from the IdP. alice (viewers), bob (editors), carol (no group) and an admin
 * against the fake backend: own groups for everyone, directory + links for admins only,
 * advisory profile filtering, and the non-blocking "IdP unavailable" banner.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { clearSent, emit, navigateTo, open, sent } from './helpers'

const issuer = {
  id: 'rauthy',
  issuer_url: 'https://auth.example.com/auth/v1',
  client_id: 'fathom',
  scopes: ['openid'],
  ca_bundle_path: null,
  username_claim: 'preferred_username',
  groups_claim: 'groups',
  username_prefix: 'oidc:',
  groups_prefix: 'oidc:',
  admin_api_base: null,
}

const roleMap = {
  viewers_group: 'fathom-viewers',
  editors_group: 'fathom-editors',
  admins_group: 'fathom-admins',
}

function ident(username: string, groups: string[]) {
  return {
    issuer_id: 'rauthy',
    sub: `${username}-sub`,
    username,
    email: `${username}@fathom.test`,
    groups,
    expires_at: 4102444800,
    state: { type: 'SignedIn' },
  }
}

const profile = (id: string, alias: string) => ({
  id,
  contextAlias: alias,
  apiServer: `https://${alias}.example.com:6443`,
  defaultNamespace: null,
  rev: 1,
  sharedBy: 'admin',
  updatedAt: '2025-01-01T00:00:00Z',
  removed: false,
})

async function signInAs(page: Page, username: string, groups: string[]) {
  await open(page)
  await emit(
    page,
    batch([
      system(10, {
        type: 'OidcConfigLoaded',
        data: {
          config: { issuers: [issuer], contexts: [], role_map: roleMap },
          identities: [ident(username, groups)],
        },
      } as never),
    ]),
  )
}

async function goToTeams(page: Page) {
  await navigateTo(page, 'Teams', 'Team')
  await expect(page.getByTestId('teams-view')).toBeVisible()
}

test.describe('033 US3 teams from the IdP', () => {
  test('alice sees only her own groups and an advisory-filtered profile list', async ({ page }) => {
    await signInAs(page, 'alice', ['fathom-viewers'])
    await emit(
      page,
      batch([
        system(11, {
          type: 'TeamLinksUpdated',
          data: {
            links: [
              { group: 'fathom-editors', profile_id: 'p-edit' },
              { group: 'fathom-viewers', profile_id: 'p-view' },
            ],
          },
        } as never),
        system(12, {
          type: 'ConnectionProfilesUpdated',
          data: {
            adopted: [],
            available: [profile('p-edit', 'edit-cluster'), profile('p-view', 'view-cluster')],
          },
        } as never),
      ]),
    )
    await navigateTo(page, 'Connection Profiles', 'Team')
    const rows = page.getByTestId('teams-available-row')
    await expect(rows).toHaveCount(1)
    await expect(rows.first()).toContainText('view-cluster')
    await expect(page.getByTestId('teams-advisory-filter')).toContainText('advisory')
    await page.getByTestId('teams-advisory-toggle').click()
    await expect(rows).toHaveCount(2)
  })

  test('alice sees her own groups and no admin controls', async ({ page }) => {
    await signInAs(page, 'alice', ['fathom-viewers'])
    await goToTeams(page)
    await expect(page.getByTestId('teams-own-groups')).toContainText('fathom-viewers')
    await expect(page.getByTestId('teams-admin')).toHaveCount(0)
    await expect(page.getByTestId('teams-admin-key-input')).toHaveCount(0)
  })

  test('bob (editor) sees his group, no admin directory', async ({ page }) => {
    await signInAs(page, 'bob', ['fathom-editors'])
    await goToTeams(page)
    await expect(page.getByTestId('teams-own-groups')).toContainText('fathom-editors')
    await expect(page.getByTestId('teams-admin')).toHaveCount(0)
  })

  test('carol (no group) sees the no-team state', async ({ page }) => {
    await signInAs(page, 'carol', [])
    await goToTeams(page)
    await expect(page.getByTestId('teams-no-groups')).toBeVisible()
    await expect(page.getByTestId('teams-admin')).toHaveCount(0)
  })

  test('admin: directory, admin key, links and IdP-unavailable banner', async ({ page }) => {
    await signInAs(page, 'root', ['fathom-admins'])
    await clearSent(page)
    await goToTeams(page)
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'IdpRefreshDirectory'))
      .toMatchObject({ data: { issuer_id: 'rauthy' } })

    await emit(
      page,
      batch([
        system(11, {
          type: 'IdpDirectoryUpdated',
          data: {
            issuer_id: 'rauthy',
            users: [
              {
                sub: 'a',
                username: 'alice',
                email: 'alice@fathom.test',
                groups: ['fathom-viewers'],
                enabled: true,
              },
              {
                sub: 'b',
                username: 'bob',
                email: 'bob@fathom.test',
                groups: ['fathom-editors'],
                enabled: true,
              },
              { sub: 'c', username: 'carol', email: null, groups: [], enabled: true },
            ],
            groups: [
              { name: 'fathom-viewers', member_count: 1 },
              { name: 'fathom-editors', member_count: 1 },
            ],
            fetched_at: 100,
            stale: false,
            error: null,
          },
        } as never),
        system(12, {
          type: 'ConnectionProfilesUpdated',
          data: { adopted: [], available: [profile('p1', 'prod')] },
        } as never),
      ]),
    )
    await expect(page.getByTestId('teams-directory-users')).toHaveAttribute('data-user-count', '3')
    await expect(page.getByTestId('teams-user-bob')).toContainText('fathom-editors')
    await page.getByTestId('teams-user-filter').fill('carol')
    await expect(page.getByTestId('teams-directory-users')).toHaveAttribute('data-user-count', '1')
    await page.getByTestId('teams-user-filter').fill('')

    // Save the read-only admin key: write-only command, then a refresh.
    await clearSent(page)
    await page.getByTestId('teams-admin-key-input').fill('fathom-read$' + 'x'.repeat(64))
    await page.getByTestId('teams-admin-key-save').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'IdpSetAdminKey'))
      .toMatchObject({ data: { issuer_id: 'rauthy' } })
    await expect(page.getByTestId('teams-admin-key-input')).toHaveValue('')

    // Link a group to a profile, then unlink it.
    await clearSent(page)
    await page.getByTestId('teams-link-group').selectOption('fathom-editors')
    await page.getByTestId('teams-link-profile').selectOption('p1')
    await page.getByTestId('teams-link-add').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'LinkTeamProfile'))
      .toEqual({ type: 'LinkTeamProfile', data: { group: 'fathom-editors', profile_id: 'p1' } })
    await emit(
      page,
      batch([
        system(13, {
          type: 'TeamLinksUpdated',
          data: { links: [{ group: 'fathom-editors', profile_id: 'p1' }] },
        } as never),
      ]),
    )
    await page.getByTestId('teams-unlink-fathom-editors-p1').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'UnlinkTeamProfile'))
      .toEqual({
        type: 'UnlinkTeamProfile',
        data: { group: 'fathom-editors', profile_id: 'p1' },
      })

    // IdP goes away: last-good data stays, a non-blocking banner explains.
    await emit(
      page,
      batch([
        system(14, {
          type: 'IdpDirectoryUpdated',
          data: {
            issuer_id: 'rauthy',
            users: [{ sub: 'a', username: 'alice', email: null, groups: [], enabled: true }],
            groups: [],
            fetched_at: 100,
            stale: true,
            error: 'IdP unreachable (users): connection refused',
          },
        } as never),
      ]),
    )
    await expect(page.getByTestId('teams-idp-unavailable')).toContainText('connection refused')
    await expect(page.getByTestId('teams-user-alice')).toBeVisible()
    await expect(page.getByTestId('teams-own-groups')).toContainText('fathom-admins')
  })

  test('role-forbidden publish is explained', async ({ page }) => {
    await signInAs(page, 'alice', ['fathom-viewers'])
    await navigateTo(page, 'Connection Profiles', 'Team')
    await emit(
      page,
      batch([
        system(11, {
          type: 'TeamPublishRejected',
          data: {
            rejection: {
              kind: 'roleForbidden',
              reason: 'publishing team data needs membership of fathom-editors',
            },
          },
        } as never),
      ]),
    )
    await expect(page.getByTestId('teams-publish-rejection')).toContainText('Not permitted')
  })
})
