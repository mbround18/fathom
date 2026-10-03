/**
 * 048-docker-management US1: see what is actually running.
 *
 * The six acceptance scenarios of the story, and between them one claim: this scene never answers
 * "nothing is running" to a question nobody asked. A runtime that is not installed, one that is
 * installed and stopped, one whose socket refuses, and one that is working are four different
 * facts, and only the last of them can produce an empty list. Each of the first three is asserted
 * on its own words, because the defect being prevented is the familiar one of a probe failure
 * rendering as an empty inventory.
 *
 * The stale case is the one worth reading twice. When the daemon stops answering mid-scene, the
 * last true reading stays on screen with the reason and the time it was read beside it (FR-003).
 * Blanking the list would throw away the only information the operator has.
 */
import { expect, test, type Page } from '@playwright/test'

import { PERMISSION_DENIED } from '../fixtures/containers'
import { expectNoHorizontalScroll, open, requestIdOf, sentTypes } from './helpers'

/** The group and its first section share the label "Containers", so the click is scoped by row. */
async function openContainers(page: Page, label = 'Containers'): Promise<void> {
  const nav = page.getByRole('navigation', { name: 'Sections' })
  if (!(await nav.isVisible())) {
    await page.getByRole('button', { name: 'Open sections' }).click()
  }
  await expect(nav).toBeVisible()
  const toggle = nav.locator('button[aria-expanded]', { hasText: 'Containers' }).first()
  if ((await toggle.getAttribute('aria-expanded')) === 'false') await toggle.click()
  await nav.locator('li button', { hasText: label }).first().click()
}

/** The navigator's Containers group, which exists only when a runtime does. */
const containersGroup = (page: Page) =>
  page
    .getByRole('navigation', { name: 'Sections' })
    .locator('button[aria-expanded]', { hasText: 'Containers' })

/** Wait for the one-shot presence probe to have been answered. */
async function probed(page: Page): Promise<void> {
  await expect.poll(async () => await sentTypes(page)).toContain('RefreshContainerRuntimePresence')
}

test.describe('US1 container inventory', () => {
  test('the section is absent when no runtime is installed', async ({ page }) => {
    await open(page, 'containers-absent')
    await probed(page)
    // Absent because the probe answered "nothing here", not because the probe has not run:
    // waiting for the answer first is what makes this assertion mean anything (FR-002).
    await expect(containersGroup(page)).toHaveCount(0)
    // And nothing was asked of a runtime that does not exist.
    expect(await sentTypes(page)).not.toContain('RefreshContainerInventory')
  })

  test('a stopped runtime says so, in those words, and names itself', async ({ page }) => {
    await open(page, 'containers-stopped')
    await probed(page)
    await openContainers(page)
    const state = page.locator('[data-pane-state="runtime-stopped"]')
    await expect(state).toBeVisible()
    await expect(state).toContainText('Docker is installed and not running')
    await expect(page.getByTestId('container-group-list')).toHaveCount(0)
    // "No containers" is the one answer a stopped runtime must never produce (FR-003). The
    // heading is where that answer would appear; the detail paragraph is allowed to use the
    // words, because what it uses them to say is that this is not that.
    await expect(state.locator('p').first()).not.toContainText(/no containers/i)
  })

  test('projects group with their member and running counts', async ({ page }) => {
    await open(page, 'containers')
    await probed(page)
    await openContainers(page)
    await expect(page.getByTestId('containers-scene')).toBeVisible()

    // Three compose projects, each with what it has and how much of it is up.
    await expect(page.getByTestId('containers-group-counts-shop')).toHaveText('2 running of 2')
    await expect(page.getByTestId('containers-group-counts-blog')).toHaveText('1 running of 2')
    await expect(page.getByTestId('containers-group-counts-api')).toHaveText('0 running of 1')

    // The rows themselves, with the runtime's own status line.
    await expect(page.getByTestId('container-row-shop-web')).toBeVisible()
    await expect(page.getByTestId('container-row-blog-worker')).toContainText('Exited (137)')
    await expect(page.getByTestId('container-row-api-app')).toHaveAttribute(
      'data-state',
      'restarting',
    )

    // Nothing is adopted in this scenario, and the heading says what that means for the operator.
    await expect(page.getByTestId('containers-group-adoption-shop')).toHaveText('Read-only')
    await expectNoHorizontalScroll(page)
  })

  test('containers with no compose project are named as such, not left blank', async ({ page }) => {
    await open(page, 'containers')
    await probed(page)
    await openContainers(page)
    const unowned = page.getByTestId('containers-group-unowned')
    await expect(unowned).toBeVisible()
    await expect(unowned).toContainText('No compose project')
    await expect(page.getByTestId('containers-group-counts-unowned')).toHaveText('3 running of 3')
    await expect(page.getByTestId('container-row-scratch')).toBeVisible()
  })

  test('a runtime that goes away mid-scene marks the list stale instead of blanking it', async ({
    page,
  }) => {
    await open(page, 'containers-lost')
    await probed(page)
    await openContainers(page)
    await expect(page.getByTestId('container-row-shop-web')).toBeVisible()
    await expect(page.getByTestId('containers-scene')).toHaveAttribute('data-stale', 'false')

    await page.getByTestId('containers-refresh').click()
    // The id the fake actually assigned, read after the click: predicting it beforehand races
    // whatever else the app is sending.
    expect(await requestIdOf(page, 'RefreshContainerInventory')).toBeGreaterThan(0)

    const stale = page.getByTestId('containers-stale')
    await expect(stale).toBeVisible()
    await expect(stale).toContainText('no longer being updated')
    await expect(stale).toContainText(PERMISSION_DENIED)
    await expect(page.getByTestId('containers-scene')).toHaveAttribute('data-stale', 'true')
    // The point of the whole test: the last reading is still there to read.
    await expect(page.getByTestId('container-row-shop-web')).toBeVisible()
    await expect(page.getByTestId('containers-as-of')).toBeVisible()
  })

  test('a refused socket is named, with what to do about it', async ({ page }) => {
    await open(page, 'containers-unreachable')
    await probed(page)
    await openContainers(page)
    const detail = page.getByTestId('containers-runtime-detail')
    await expect(detail).toBeVisible()
    // The runtime's own sentence, which names the socket.
    await expect(detail).toHaveText(PERMISSION_DENIED)
    const remediation = page.getByTestId('containers-remediation')
    await expect(remediation).toContainText('unix:///var/run/docker.sock')
    await expect(remediation).toContainText('usermod -aG docker')
    await expect(page.getByTestId('container-group-list')).toHaveCount(0)
  })
})
