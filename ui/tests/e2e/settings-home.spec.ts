/**
 * 043 US3 (T030): Settings have one home, reachable from the header. The header "Settings" menu
 * sits immediately left of the sign-in chip at the right end of the header; on phone its pages
 * fold into the overflow menu. Every Settings page opens without a cluster, and Local clusters
 * is absent in hosted mode.
 */
import { expect, test, type Page } from '@playwright/test'

import { isPhone, listPane, loadClusterSettings, open, openHosted } from './helpers'

const PAGES: readonly [label: string, section: string][] = [
  ['Cluster settings', 'integrations/cluster-settings'],
  ['Sign-in', 'settings/sign-in'],
  ['Team', 'team/teams'],
  ['Connection profiles', 'team/profiles'],
  ['App settings', 'settings/app'],
  ['Local clusters', 'settings/local-clusters'],
  ['Audit log', 'audit-log'],
]

/** Open the Settings pages menu: the header button, or the overflow menu on phone. */
async function openSettingsMenu(page: Page, phone: boolean): Promise<void> {
  const trigger = page.getByTestId(phone ? 'overflow-menu' : 'settings-menu')
  const menu = page.getByRole('menu')
  // Let a previous menu finish closing; a click during its exit animation is swallowed.
  await expect(menu).toHaveCount(0)
  await expect(async () => {
    if ((await menu.count()) === 0) await trigger.click()
    await expect(menu).toBeVisible({ timeout: 1_000 })
  }).toPass()
}

async function openSettingsPage(page: Page, phone: boolean, label: string): Promise<void> {
  await openSettingsMenu(page, phone)
  await page.getByRole('menuitem', { name: label, exact: true }).click()
}

test.describe('header Settings entry', () => {
  test('sits immediately before the sign-in chip, at the right end of the header', async ({
    page,
  }, info) => {
    test.skip(isPhone(info), 'phone keeps the Settings pages in the overflow menu')
    await open(page)
    const settings = page.getByTestId('settings-menu')
    await expect(settings).toBeVisible()
    await expect(settings).toHaveRole('button')
    await expect(settings).toHaveAccessibleName('Settings')
    const order = await settings.evaluate((el) => ({
      next: (el.nextElementSibling as HTMLElement | null)?.dataset.testid,
      last: (el.parentElement?.lastElementChild as HTMLElement | null)?.dataset.testid,
      inActions: el.parentElement?.getAttribute('aria-label'),
    }))
    expect(order).toEqual({ next: 'identity-chip', last: 'identity-chip', inActions: 'Actions' })
    // Right end: after the context and namespace pickers, and in keyboard order too.
    const picker = await page.getByTestId('palette-open').boundingBox()
    const box = await settings.boundingBox()
    expect(box?.x ?? 0).toBeGreaterThan(picker?.x ?? Infinity)
    await settings.focus()
    await page.keyboard.press('Tab')
    await expect(page.getByTestId('identity-chip')).toBeFocused()
  })

  test('the menu lists every Settings page, and each opens its page', async ({ page }, info) => {
    const phone = isPhone(info)
    await open(page)
    await openSettingsMenu(page, phone)
    const items = page.getByRole('menuitem')
    for (const [label] of PAGES) {
      await expect(items.filter({ hasText: new RegExp(`^${label}$`) })).toHaveCount(1)
    }
    await page.keyboard.press('Escape')
    for (const [label, section] of PAGES) {
      await openSettingsPage(page, phone, label)
      await expect(listPane(page, section)).toBeVisible()
    }
  })

  test('on phone the Settings pages are in the overflow menu, not the bar', async ({
    page,
  }, info) => {
    test.skip(!isPhone(info), 'phone only')
    await open(page)
    await expect(page.getByTestId('settings-menu')).toHaveCount(0)
    await page.getByTestId('overflow-menu').click()
    await expect(page.getByRole('menuitem', { name: 'Cluster settings' })).toBeVisible()
    await expect(page.getByRole('menuitem', { name: 'Audit log' })).toBeVisible()
  })

  test('from tablet up the overflow menu no longer carries the Settings pages', async ({
    page,
  }, info) => {
    test.skip(isPhone(info), 'phone folds them into the overflow menu')
    await open(page)
    await expect(page.getByTestId('overflow-menu')).toHaveCount(0)
  })
})

test.describe('Settings pages', () => {
  test('Cluster settings keeps only per-context settings; the rest moved', async ({
    page,
  }, info) => {
    const phone = isPhone(info)
    await open(page)
    await openSettingsPage(page, phone, 'Cluster settings')
    await loadClusterSettings(page)
    const cluster = page.getByTestId('teams-cluster-settings-panel')
    await expect(cluster.getByTestId('teams-metrics-source')).toBeVisible()
    await expect(cluster.getByTestId('cluster-settings-daemon')).toBeVisible()
    await expect(cluster.getByTestId('appearance-settings')).toHaveCount(0)
    await expect(cluster.getByTestId('ai-assist-settings')).toHaveCount(0)
    await expect(cluster.getByTestId('mcp-available-agents')).toHaveCount(0)
    await expect(cluster.getByTestId('kind-clusters')).toHaveCount(0)

    await openSettingsPage(page, phone, 'App settings')
    const app = page.getByTestId('app-settings-panel')
    await expect(app.getByTestId('appearance-settings')).toBeVisible()
    await expect(app.getByTestId('ai-assist-settings')).toBeVisible()
    await expect(app.getByTestId('mcp-available-agents')).toBeVisible()

    await openSettingsPage(page, phone, 'Local clusters')
    await expect(
      page.getByTestId('local-clusters-panel').getByTestId('kind-clusters'),
    ).toBeVisible()
  })

  test('every Settings page opens with no cluster; Cluster settings says it needs a context', async ({
    page,
  }, info) => {
    const phone = isPhone(info)
    await open(page, 'no-kubeconfig')
    for (const [label, section] of PAGES) {
      await openSettingsPage(page, phone, label)
      const pane = listPane(page, section)
      await expect(pane).toBeVisible()
      // Only Cluster settings explains the missing kubeconfig (inside its own page); the other
      // pages are clusterless and never swap in the per-pane first-run state.
      await expect(pane.getByTestId('first-run')).toHaveCount(label === 'Cluster settings' ? 1 : 0)
    }
    await openSettingsPage(page, phone, 'Cluster settings')
    await expect(page.getByTestId('cluster-settings-needs-context')).toContainText(
      'belong to a context',
    )
    await page.getByTestId('cluster-settings-open-local-clusters').click()
    await expect(listPane(page, 'settings/local-clusters')).toBeVisible()
    await expect(page.getByTestId('kind-clusters')).toBeVisible()
  })

  test('Local clusters is absent in hosted mode', async ({ page }, info) => {
    const phone = isPhone(info)
    await openHosted(page)
    await openSettingsMenu(page, phone)
    await expect(page.getByRole('menuitem', { name: 'Cluster settings' })).toBeVisible()
    await expect(page.getByRole('menuitem', { name: 'Local clusters' })).toHaveCount(0)
  })
})
