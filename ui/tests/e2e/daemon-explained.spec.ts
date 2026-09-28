/**
 * 043 US4 (T050): the in-cluster daemon is explained. The chip always names itself, the switch
 * says what on and off do (feedback #12), and "While you were away" is now "Cluster history",
 * with a subtitle and, when the daemon is not available, an explanation of how to turn it on.
 */
import { expect, test, type Page, type TestInfo } from '@playwright/test'

import { daemonBatch, daemonStatus, entry } from '../fixtures/daemon'
import {
  clearSent,
  emit,
  isPhone,
  listPane,
  loadClusterSettings,
  navigateTo,
  open,
  sent,
} from './helpers'

const SWITCH_LABEL = "Use Fathom's in-cluster daemon"
const SWITCH_DESCRIPTION =
  'On: Fathom checks this cluster for its optional, read-only daemon and, if found, uses it for cluster history and a faster topology. Off: Fathom never contacts it. Nothing is installed either way.'
const SUBTITLE = 'Changes the in-cluster daemon recorded, including while Fathom was closed.'

/** Phone and tablet show the chip as an icon; laptop and up show its words. */
const narrow = (i: TestInfo): boolean =>
  i.project.name.endsWith('-phone') || i.project.name.endsWith('-tablet')

const chip = (page: Page) => page.getByTestId('daemon-chip')

test.describe('daemon chip', () => {
  test('is named "Daemon: <state>" at every width, and says so at laptop and up', async ({
    page,
  }, info) => {
    await open(page)
    await expect(chip(page)).toHaveAccessibleName('Daemon: Off')
    if (narrow(info)) await expect(chip(page)).not.toContainText('Daemon')
    else await expect(chip(page)).toHaveText(/^Daemon:\s*○\s*Off$/)
    await emit(page, daemonStatus('available'))
    await expect(chip(page)).toHaveAccessibleName('Daemon: Connected')
    await expect(chip(page)).toHaveAccessibleDescription(/in-cluster daemon/)
  })

  test('the switch is named for what it controls and says what on and off do', async ({ page }) => {
    await open(page)
    await chip(page).click()
    const dialog = page.getByTestId('daemon-dialog')
    const toggle = dialog.getByRole('switch', { name: SWITCH_LABEL })
    await expect(toggle).toBeVisible()
    await expect(toggle).toHaveAccessibleDescription(SWITCH_DESCRIPTION)
    await expect(dialog).toContainText('Fathom never installs it')
    await clearSent(page)
    await toggle.click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'SetDaemonPreference'))
      .toMatchObject({ data: { context: 'kind-a', mode: 'auto' } })
  })
})

test.describe('Cluster history', () => {
  test('the navigator entry explains why it is empty and how to turn it on', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Cluster history', 'Cluster')
    const pane = listPane(page, 'cluster-history')
    await expect(pane.getByRole('heading', { name: 'Cluster history' })).toBeVisible()
    await expect(pane.getByTestId('cluster-history-subtitle')).toHaveText(SUBTITLE)
    const why = pane.getByTestId('cluster-history-unavailable')
    await expect(why).toContainText(SWITCH_LABEL)
    await expect(why).toContainText('Settings › Cluster settings')
    await pane.getByTestId('cluster-history-open-settings').click()
    await expect(listPane(page, 'integrations/cluster-settings')).toBeVisible()
  })

  test('with the daemon available it opens with the subtitle and lists the history', async ({
    page,
  }) => {
    await open(page)
    await emit(page, daemonStatus('available'))
    await emit(page, daemonBatch(1, [entry(1)]))
    await navigateTo(page, 'Cluster history', 'Cluster')
    const pane = listPane(page, 'cluster-history')
    await expect(pane.getByTestId('cluster-history-subtitle')).toHaveText(SUBTITLE)
    await expect(pane.getByRole('list', { name: 'Cluster history' })).toBeVisible()
    await expect(pane.getByTestId('daemon-timeline-entry')).toHaveCount(1)
  })

  test('the daemon dialog opens Cluster history', async ({ page }) => {
    await open(page)
    await emit(page, daemonStatus('available'))
    await chip(page).click()
    await expect(page.getByTestId('daemon-dialog')).toContainText(SUBTITLE)
    await page.getByRole('button', { name: 'Open Cluster history' }).click()
    await expect(listPane(page, 'cluster-history')).toBeVisible()
  })

  // T041 (US5): the dock renders `dockTabs(profile.dock, daemonAvailable)`, which adds the
  // Cluster history tab only while the daemon is available.
  test('the dock shows a Cluster history tab only while the daemon is available', async ({
    page,
  }, info) => {
    test.skip(narrow(info), 'no dock below laptop width')
    await open(page)
    const dock = page.getByTestId('dock-tabs')
    await expect(dock.getByRole('tab', { name: 'Cluster history' })).toHaveCount(0)
    await emit(page, daemonStatus('available'))
    await dock.getByRole('tab', { name: 'Cluster history' }).click()
    await expect(dock.getByTestId('cluster-history-subtitle')).toHaveText(SUBTITLE)
  })
})

test('Settings › Cluster settings has the same switch and explanation', async ({ page }, info) => {
  await open(page)
  if (isPhone(info)) await page.getByTestId('overflow-menu').click()
  else await page.getByTestId('settings-menu').click()
  await page.getByRole('menuitem', { name: 'Cluster settings' }).click()
  await loadClusterSettings(page)
  const section = page.getByTestId('cluster-settings-daemon')
  const toggle = section.getByRole('switch', { name: SWITCH_LABEL })
  await expect(toggle).toHaveAccessibleDescription(SWITCH_DESCRIPTION)
  await expect(toggle).not.toBeChecked()
  await clearSent(page)
  await toggle.click()
  await expect
    .poll(async () => (await sent(page)).find((c) => c.type === 'SetDaemonPreference'))
    .toMatchObject({ data: { context: 'kind-a', mode: 'auto' } })
  await expect(toggle).toBeChecked()
})
