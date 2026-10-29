/**
 * Spec 062 US4 (T071): Watchtower labels keep meaning what they meant, and Watchtower itself is
 * noticed.
 *
 * - `enable=false`, `monitor-only=true`, `scope=media` and `no-pull=true` show their effective
 *   policy and where it comes from, on the Updates page and on the stack page.
 * - Hook and `depends-on` labels are shown "not run" with the reason.
 * - A running Watchtower container pauses Auto; the banner names it, and an admin's answer
 *   ("stopped" or "only monitors") confirmed with the passkey lifts the pause.
 */
import type { Page } from '@playwright/test'

import type { ServiceUpdate, UpdatesView } from '../../../src/protocol/generated'
import { expect, fixtures as f, test, type FakeBackend } from '../../support/fake-backend'
import { seedPasskey, virtualAuthenticator } from '../../support/webauthn'

const P = 'com.centurylinklabs.watchtower'
const HOOK = 'lifecycle hooks run commands in the container; Depths only runs commands through the guarded shell'
const row = (page: Page, id: string) => page.locator(`[data-update-row="${id}"]`)

/** The media stack as the server reads it with `scope: media` set. */
function mediaServices(): ServiceUpdate[] {
  const su = (service: string, state: ServiceUpdate['state'], over: Partial<ServiceUpdate>) => f.serviceUpdate('media', service, state, over)
  return [
    su('jellyfin', 'excluded', {
      effective: {
        policy: 'off',
        source: { kind: 'label', rule: 'enable' },
        excluded: { kind: 'label_disabled' },
        labels: [{ key: `${P}.enable`, value: 'false', effect: { kind: 'applied' } }],
      },
    }),
    su('sonarr', 'available', {
      effective: {
        policy: 'notify',
        source: { kind: 'label', rule: 'monitor_only' },
        excluded: null,
        labels: [
          { key: `${P}.monitor-only`, value: 'true', effect: { kind: 'applied' } },
          { key: `${P}.scope`, value: 'media', effect: { kind: 'applied' } },
        ],
      },
    }),
    su('radarr', 'up_to_date', {
      effective: {
        policy: 'auto',
        source: { kind: 'stack_policy' },
        excluded: null,
        labels: [
          { key: `${P}.scope`, value: 'media', effect: { kind: 'applied' } },
          { key: `${P}.lifecycle.pre-update`, value: '/backup.sh', effect: { kind: 'not_run', reason: HOOK } },
          { key: `${P}.depends-on`, value: 'db', effect: { kind: 'not_run', reason: 'order comes from the compose file' } },
        ],
      },
    }),
    su('prowlarr', 'excluded', {
      effective: {
        policy: 'off',
        source: { kind: 'label', rule: 'no_pull' },
        excluded: { kind: 'no_pull' },
        labels: [
          { key: `${P}.scope`, value: 'media', effect: { kind: 'applied' } },
          { key: `${P}.no-pull`, value: 'true', effect: { kind: 'applied' } },
        ],
      },
    }),
  ]
}

/** The media stack with the four services above, so the stack page lists them. */
function mediaStack() {
  const names = ['jellyfin', 'sonarr', 'radarr', 'prowlarr']
  return f.stack('media', {
    services: names.map((n) => ({
      name: n,
      image: `lscr.io/linuxserver/${n}:latest`,
      containers: [f.container({ name: `media-${n}-1`, project: 'media', service: n, state: 'running', status: 'Up 3 hours' })],
    })),
    running_services: names.length,
    total_services: names.length,
  })
}

function seed(backend: FakeBackend, over: Partial<UpdatesView> = {}): UpdatesView {
  backend.stacks = f.stackList([f.stack('web'), mediaStack()])
  const view = f.updatesView({ settings: f.updateSettings({ checks_enabled: true, scope: 'media' }), ...over })
  view.services = [...view.services.filter((s) => s.stack !== 'media'), ...mediaServices()]
  backend.setUpdates(view)
  return view
}

function withWatchtower(): Partial<UpdatesView> {
  return {
    watchtower: [{ id: 'abc123def456', name: 'watchtower', image: 'containrrr/watchtower:latest', acknowledged: null }],
    auto_paused: { kind: 'watchtower_running', containers: ['watchtower'] },
  }
}

for (const width of [360, 1280]) {
  test(`at ${String(width)} px: each label shows its effective policy and source`, async ({ page, backend }) => {
    await page.setViewportSize({ width, height: 900 })
    seed(backend)
    await page.goto('/admin/updates')

    const jellyfin = row(page, 'media/jellyfin')
    await expect(jellyfin.locator('[data-field="policy"]')).toHaveText('Off · enable label')
    await expect(jellyfin.locator('[data-label="enable"]')).toHaveText('enable=false applied')
    await expect(jellyfin.locator('[data-field="reason"]')).toContainText('enable=false label')

    const sonarr = row(page, 'media/sonarr')
    await expect(sonarr.locator('[data-field="policy"]')).toHaveText('Notify · monitor-only label')
    await expect(sonarr.locator('[data-label="monitor-only"]')).toHaveText('monitor-only=true applied')
    await expect(sonarr.locator('[data-label="scope"]')).toHaveText('scope=media applied')

    const prowlarr = row(page, 'media/prowlarr')
    await expect(prowlarr.locator('[data-field="policy"]')).toHaveText('Off · no-pull label')
    await expect(prowlarr.locator('[data-label="no-pull"]')).toHaveText('no-pull=true applied')

    const radarr = row(page, 'media/radarr')
    await expect(radarr.locator('[data-field="policy"]')).toHaveText('Auto · stack policy')
    await expect(radarr.locator('[data-label="lifecycle.pre-update"]')).toHaveAttribute('data-effect', 'not_run')
    await expect(radarr.locator('[data-label="lifecycle.pre-update"]')).toContainText(`not run: ${HOOK}`)
    await expect(radarr.locator('[data-label="depends-on"]')).toHaveText('depends-on=db not run: order comes from the compose file')

    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)
    expect(overflow).toBeLessThanOrEqual(0)
  })
}

test('the stack page shows the same policy, source and labels', async ({ page, backend }) => {
  seed(backend)
  backend.details.media = f.detail(mediaStack(), { updates: mediaServices() })
  await page.goto('/stacks/media')
  const radarr = page.locator('[data-service="radarr"] [data-field="update"]')
  await expect(radarr.locator('[data-field="policy"]')).toHaveText('Auto · stack policy')
  await expect(radarr.locator('[data-label="depends-on"]')).toContainText('not run: order comes from the compose file')
  const jellyfin = page.locator('[data-service="jellyfin"] [data-field="update"]')
  await expect(jellyfin.locator('[data-field="policy"]')).toHaveText('Off · enable label')
})

test('the banner names Watchtower, and "stopped" confirmed with the passkey lifts the pause', async ({ page, backend }) => {
  const auth = await virtualAuthenticator(page)
  const phone = await seedPasskey(auth, 'Phone')
  seed(backend, withWatchtower())
  backend.passkeys.enable({ items: [phone], verified: true, stale: true })
  await page.goto('/admin/updates')

  const banner = page.getByTestId('updates-watchtower')
  await expect(banner).toContainText('Auto updates are paused: Watchtower is running (watchtower).')
  await expect(banner.locator('[data-watchtower="watchtower"]')).toContainText('containrrr/watchtower:latest')
  await banner.locator('[data-action="ack-stopped"]').click()
  const preview = await backend.waitForCmd('PreviewUpdateChange')
  expect(preview.data).toEqual({ kind: 'ack_watchtower', data: { container_id: 'abc123def456', answer: 'stopped' } })

  const dialog = page.getByTestId('updates-settings-dialog')
  await expect(dialog).toContainText('Resume Auto updates')
  await expect(dialog.locator('[data-field="summary"]')).toContainText('you stopped it, or will stop it')
  await dialog.locator('[data-action="confirm"]').click()
  const confirm = await backend.waitForCmd('Confirm')
  expect(confirm.data.assertion?.credential.id).toBe(phone.id)
  await expect(banner).toHaveCount(0)
  expect(backend.updates?.auto_paused).toBeNull()
})

test('"only monitors" lifts the pause too', async ({ page, backend }) => {
  seed(backend, withWatchtower())
  await page.goto('/admin/updates')
  await page.getByTestId('updates-watchtower').locator('[data-action="ack-monitor-only"]').click()
  const dialog = page.getByTestId('updates-settings-dialog')
  await expect(dialog.locator('[data-field="summary"]')).toContainText('it only monitors (WATCHTOWER_MONITOR_ONLY)')
  await dialog.locator('[data-action="confirm"]').click()
  await expect(page.getByTestId('updates-watchtower')).toHaveCount(0)
  expect(backend.updates?.watchtower[0]?.acknowledged).toBe('monitor_only')
})
