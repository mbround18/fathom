/**
 * 043 US1 (T010): with no usable kubeconfig or no current context, one setup screen replaces the
 * navigator, the panes and the dock. It lists every context verbatim, remembers the pick in
 * Fathom's UI config, keeps feedback #1's reload answer, and steps aside for Settings.
 */
import { expect, test, type Page } from '@playwright/test'

import { MALFORMED_DETAIL, NO_KUBECONFIG_PATH, PODS_KIND_B, UNREACHABLE_DETAIL } from '../fixtures'
import { clearSent, expectRowCount, isPhone, open, sent, sentTypes } from './helpers'

const setup = (page: Page) => page.getByTestId('setup-screen')
const contextList = (page: Page) => page.getByRole('listbox', { name: 'Contexts' })
const filterBox = (page: Page) => page.getByRole('searchbox', { name: 'Filter contexts' })

async function setContexts(page: Page) {
  return (await sent(page)).filter((c) => c.type === 'SetContext')
}

test.describe('043 setup screen', () => {
  test('no current context: only the setup screen fills the main area', async ({ page }) => {
    await open(page, 'no-current-context')
    await expect(setup(page)).toHaveAttribute('data-setup-state', 'pick')
    const main = page.locator('#main-content')
    await expect(main.locator(':scope > *')).toHaveCount(1)
    await expect(main.locator(':scope > [data-testid="setup-screen"]')).toHaveCount(1)
    await expect(page.locator('[data-slot="pane"]')).toHaveCount(0)
    await expect(page.getByRole('navigation', { name: 'Sections' })).toHaveCount(0)
    // The top bar and the status bar stay.
    await expect(page.getByRole('banner')).toBeVisible()
    await expect(page.getByTestId('status-bar')).toBeVisible()
    // Every context, verbatim, and where they came from.
    await expect(contextList(page).getByRole('option')).toHaveText(['kind-a', 'kind-b'])
    await expect(setup(page).getByTestId('setup-paths')).toContainText(NO_KUBECONFIG_PATH)
  })

  test('the filter narrows the list and the keyboard picks', async ({ page }) => {
    await open(page, 'no-current-context')
    await expect(filterBox(page)).toBeFocused()
    await filterBox(page).fill('b')
    await expect(contextList(page).getByRole('option')).toHaveText(['kind-b'])
    await filterBox(page).fill('nope')
    await expect(contextList(page).getByRole('option')).toHaveCount(0)
    await expect(setup(page).getByTestId('setup-no-match')).toContainText('nope')
    await filterBox(page).fill('')
    await filterBox(page).press('ArrowDown')
    await expect(contextList(page).getByRole('option', { name: 'kind-b' })).toHaveAttribute(
      'aria-selected',
      'true',
    )
    await clearSent(page)
    await filterBox(page).press('Enter')
    await expect
      .poll(() => setContexts(page))
      .toEqual([{ type: 'SetContext', data: { context: 'kind-b' } }])
  })

  test('a pick connects, remembers the context, and opens the shell', async ({ page }) => {
    await open(page, 'no-current-context')
    await clearSent(page)
    await contextList(page).getByRole('option', { name: 'kind-b' }).click()
    await expect
      .poll(() => setContexts(page))
      .toEqual([{ type: 'SetContext', data: { context: 'kind-b' } }])
    const save = (await sent(page, true)).find((c) => c.type === 'SaveUiConfig')
    expect(save).toMatchObject({
      type: 'SaveUiConfig',
      data: { ui: { lastSelection: { context: 'kind-b', section: 'overview' } } },
    })
    // The kubeconfig is never written.
    expect(await sentTypes(page)).not.toContain('SetCurrentContext')
    await expect(setup(page)).toHaveCount(0)
    await expect(page.locator('[data-slot="pane"]').first()).toBeVisible()
  })

  test('a failed connect keeps the screen: Retry, Details, choose another', async ({ page }) => {
    await open(page, 'setup-unreachable')
    await contextList(page).getByRole('option', { name: 'kind-a' }).click()
    await expect(setup(page)).toHaveAttribute('data-setup-state', 'failed')
    await expect(setup(page)).toContainText('Cannot reach kind-a')
    await expect(page.locator('[data-slot="pane"]')).toHaveCount(0)
    await setup(page).getByText('Details').click()
    await expect(setup(page).getByTestId('setup-detail')).toContainText(UNREACHABLE_DETAIL)

    await clearSent(page)
    await setup(page).getByTestId('setup-retry').click()
    await expect
      .poll(() => setContexts(page))
      .toEqual([{ type: 'SetContext', data: { context: 'kind-a' } }])
    await expect(setup(page)).toHaveAttribute('data-setup-state', 'failed')

    await setup(page).getByTestId('setup-choose-another').click()
    await expect(setup(page)).toHaveAttribute('data-setup-state', 'pick')
    await clearSent(page)
    await contextList(page).getByRole('option', { name: 'kind-b' }).click()
    await expect
      .poll(() => setContexts(page))
      .toEqual([{ type: 'SetContext', data: { context: 'kind-b' } }])
    await expect(setup(page)).toHaveCount(0)
    // A pick opens on the Overview.
    await expect(page.locator('[data-slot="pane"]').first()).toBeVisible()
  })

  test('missing kubeconfig: names the path, reloads with an answer, offers a local cluster', async ({
    page,
  }) => {
    await open(page, 'no-kubeconfig')
    await expect(setup(page)).toHaveAttribute('data-setup-state', 'kubeconfig')
    await expect(setup(page)).toContainText('No kubeconfig found')
    await expect(setup(page).getByTestId('setup-paths')).toContainText(NO_KUBECONFIG_PATH)
    await expect(page.getByTestId('status-notice')).toHaveCount(0)
    await clearSent(page)
    await setup(page).getByTestId('setup-reload').click()
    await expect.poll(() => sentTypes(page)).toContain('Initialize')
    // Feedback #1: an unchanged result still answers, naming where Fathom looked.
    await expect(setup(page).getByTestId('setup-reload-outcome')).toHaveText(
      `No kubeconfig found; looked in ${NO_KUBECONFIG_PATH}.`,
    )
    await expect(setup(page).getByTestId('setup-reload')).toBeEnabled()

    await setup(page).getByTestId('setup-create-kind').click()
    await expect(page.getByTestId('local-clusters-panel')).toBeVisible()
    await page.getByTestId('setup-back').click()
    await expect(setup(page)).toHaveAttribute('data-setup-state', 'kubeconfig')
  })

  test('malformed kubeconfig: the parser detail sits under Details', async ({ page }) => {
    await open(page, 'kubeconfig-malformed')
    await expect(setup(page)).toContainText('Kubeconfig could not be parsed')
    await expect(setup(page).getByTestId('setup-detail')).toBeHidden()
    await setup(page).getByText('Details').click()
    await expect(setup(page).getByTestId('setup-detail')).toContainText(MALFORMED_DETAIL)
  })

  test('empty kubeconfig: says no clusters are configured', async ({ page }) => {
    await open(page, 'kubeconfig-empty')
    await expect(setup(page)).toContainText('No clusters configured')
    await expect(setup(page).getByTestId('setup-create-kind')).toBeVisible()
  })

  test('a remembered context skips setup at the next start', async ({ page }) => {
    await open(page, 'remembered-context')
    await expect
      .poll(() => setContexts(page))
      .toEqual([{ type: 'SetContext', data: { context: 'kind-b' } }])
    await expectRowCount(page, PODS_KIND_B.length)
    await expect(setup(page)).toHaveCount(0)
  })

  test('a remembered context that was removed shows setup', async ({ page }) => {
    await open(page, 'remembered-context-removed')
    await expect(setup(page)).toHaveAttribute('data-setup-state', 'pick')
    expect(await setContexts(page)).toEqual([])
  })

  test('hosted mode never shows it', async ({ page }) => {
    await page.goto('/?scenario=no-current-context&mode=hosted&user=alice&groups=dev')
    await page.waitForFunction(() => window.__fathomFake !== undefined)
    await expect(page.getByTestId('status-bar')).toBeVisible()
    await expect(page.locator('[data-slot="pane"]').first()).toBeVisible()
    await expect(setup(page)).toHaveCount(0)
  })

  test('two columns from md up, one column on a phone', async ({ page }, info) => {
    await open(page, 'no-current-context')
    const intro = await setup(page).getByTestId('setup-intro').boundingBox()
    const list = await setup(page).getByTestId('setup-contexts').boundingBox()
    if (!intro || !list) throw new Error('setup columns not rendered')
    if (isPhone(info)) {
      expect(list.y).toBeGreaterThanOrEqual(intro.y + intro.height - 1)
    } else {
      expect(list.x).toBeGreaterThanOrEqual(intro.x + intro.width - 1)
    }
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    )
    expect(overflow).toBeLessThanOrEqual(0)
  })

  test('reloading the kubeconfig keeps the filter text', async ({ page }) => {
    await open(page, 'no-current-context')
    await filterBox(page).fill('kind-b')
    await setup(page).getByTestId('setup-reload').click()
    await expect(setup(page).getByTestId('setup-reload-outcome')).toContainText('Found 2 contexts')
    await expect(filterBox(page)).toHaveValue('kind-b')
    await expect(contextList(page).getByRole('option')).toHaveText(['kind-b'])
  })

  test('Settings from the header replaces setup until "Back to setup"', async ({ page }) => {
    await open(page, 'no-current-context')
    await page.getByTestId('overflow-menu').click()
    await page.getByRole('menuitem', { name: 'Settings' }).click()
    await expect(setup(page)).toHaveCount(0)
    await expect(page.getByTestId('setup-back')).toBeVisible()
    await expect(page.locator('[data-slot="pane"]')).toHaveCount(1)
    await page.getByTestId('setup-back').click()
    await expect(setup(page)).toHaveAttribute('data-setup-state', 'pick')
  })
})
