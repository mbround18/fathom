/**
 * Guide screenshots for image updates (spec 062 T087): the Updates list with its states, a batch
 * preview, the policy sheet, the Watchtower banner and the phone layout.
 *
 *   DEPTHS_PROMO=1 pnpm exec playwright test --project=promo tests/promo/updates.capture.spec.ts
 *
 * Fixture stacks only, against the fake backend; images land in docs/guides/depths/img/.
 */
import path from 'node:path'

import type { Page } from '@playwright/test'

import { expect, fixtures as f, test, type FakeBackend } from '../support/fake-backend'
import type { UpdatesView } from '../../src/protocol/generated/UpdatesView'

const OUT = path.resolve(import.meta.dirname, '../../../../../docs/guides/depths/img')

/** The US1 fixture plus a held, a rolled-back and a moved-back service, so the list shows them. */
function seed(backend: FakeBackend, over: Partial<UpdatesView> = {}): void {
  backend.stacks = f.stackList([f.stack('web'), f.stack('blog'), f.stack('media')])
  const view = f.updatesView(over)
  view.services = [
    ...view.services,
    f.serviceUpdate('media', 'sonarr', 'available', { reference: 'lscr.io/linuxserver/sonarr:latest' }),
    f.serviceUpdate('media', 'radarr', 'held', {
      reference: 'lscr.io/linuxserver/radarr:latest',
      reason: 'held: the new image is labelled as a breaking change',
    }),
    f.serviceUpdate('blog', 'search', 'rolled_back', {
      reference: 'getmeili/meilisearch:v1',
      reason: 'rolled back: unhealthy after 120 s',
    }),
    f.serviceUpdate('web', 'proxy', 'moved_back', { reference: 'caddy:2', reason: 'the tag now points at an older image' }),
  ]
  backend.setUpdates(view)
}

async function shot(page: Page, name: string): Promise<void> {
  await page.waitForTimeout(250)
  await page.screenshot({ path: path.join(OUT, `${name}.png`) })
}

test.use({ viewport: { width: 1440, height: 900 } })

test('updates: list, batch preview, policy sheet', async ({ page, backend }) => {
  seed(backend)
  await page.goto('/admin/updates')
  await expect(page.locator('[data-update-row="web/app"]')).toBeVisible()
  await shot(page, 'updates-list')

  await page.locator('[data-update-row="web/app"] [data-select-stack]').check()
  await page.locator('[data-update-row="blog/ghost"] [data-select-stack]').check()
  await page.locator('[data-action="preview-batch"]').click()
  const batch = page.getByTestId('updates-batch-dialog')
  await expect(batch.locator('[data-plan]')).toHaveCount(2)
  await shot(page, 'updates-batch-preview')
  await page.keyboard.press('Escape')
  await expect(batch).toHaveCount(0)

  const policies = page.getByTestId('updates-policies')
  if ((await policies.getAttribute('open')) === null) await policies.locator('summary').click()
  await policies.locator('[data-policy-row="web"] [data-action="edit-policy"]').click()
  const sheet = page.getByTestId('updates-policy-sheet')
  await sheet.locator('[data-policy-option="auto"]').check()
  await shot(page, 'updates-policy-sheet')
})

test('updates: Watchtower banner', async ({ page, backend }) => {
  seed(backend, {
    watchtower: [{ id: 'abc123def456', name: 'watchtower', image: 'containrrr/watchtower:latest', acknowledged: null }],
    auto_paused: { kind: 'watchtower_running', containers: ['watchtower'] },
  })
  await page.goto('/admin/updates')
  await expect(page.getByTestId('updates-watchtower')).toBeVisible()
  await shot(page, 'updates-watchtower')
})

test.describe('phone', () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true })

  test('updates: phone list', async ({ page, backend }) => {
    seed(backend)
    await page.goto('/admin/updates')
    await expect(page.locator('[data-update-row="web/app"]')).toBeVisible()
    await shot(page, 'updates-phone-list')
  })
})
