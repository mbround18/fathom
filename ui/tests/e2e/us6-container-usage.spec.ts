/**
 * 048-docker-management US6: what is eating the machine.
 *
 * The five acceptance scenarios, and under them one claim worth more than the figures themselves:
 * an unmeasured container is not an idle one. A row the runtime could not measure says so, and a
 * project's total says how many members it is actually the total of (FR-038) — because a sum that
 * counted the unmeasured as zero would be a smaller number with nothing on screen to give it away,
 * and the operator would stop the wrong project.
 *
 * The other claim is about cost. Sampling exists for exactly as long as the panel does, so leaving
 * the selection or the section stops it (FR-045), and the commands on the wire are what prove it:
 * a sampler nobody asked to stop is not something an operator can check for themselves.
 */
import { expect, test, type Page } from '@playwright/test'

import { ALL_CONTAINERS } from '../fixtures/containers'
import { open, sent, sentTypes } from './helpers'

function idOf(name: string): string {
  const found = ALL_CONTAINERS.find((c) => c.name === name)
  if (!found) throw new Error(`no fixture container named ${name}`)
  return found.id
}

const WEB = idOf('shop-web')
const DB = idOf('shop-db')

/** The group and its first section share the label "Containers", so the click is scoped by row. */
async function openContainers(page: Page): Promise<void> {
  const nav = page.getByRole('navigation', { name: 'Sections' })
  if (!(await nav.isVisible())) {
    await page.getByRole('button', { name: 'Open sections' }).click()
  }
  await expect(nav).toBeVisible()
  const toggle = nav.locator('button[aria-expanded]', { hasText: 'Containers' }).first()
  if ((await toggle.getAttribute('aria-expanded')) === 'false') await toggle.click()
  await nav.locator('li button', { hasText: 'Containers' }).first().click()
}

/** The scene, with the inventory on screen. */
async function scene(page: Page): Promise<void> {
  await open(page, 'containers-usage')
  await expect.poll(async () => await sentTypes(page)).toContain('RefreshContainerRuntimePresence')
  await openContainers(page)
  await expect(page.getByTestId('container-group-list')).toBeVisible()
}

/** The ids a sampling start was asked for, per command, in order. */
async function sampledIds(page: Page): Promise<string[][]> {
  const commands = await sent(page)
  return commands
    .filter((c) => c.type === 'StartContainerUsageSampling')
    .map((c) => ('data' in c ? c.data.container_ids : []))
}

test.describe('US6 container usage', () => {
  test('a project shows its members and the aggregate of the ones that reported', async ({
    page,
  }) => {
    await scene(page)
    await page.getByTestId('containers-group-shop').click()

    const panel = page.getByTestId('container-usage')
    await expect(panel).toBeVisible()
    await expect(panel).toContainText('shop-web')
    await expect(panel).toContainText('shop-db')

    // The project as one unit: one of its two members reported, and the total says so rather than
    // presenting itself as the project's whole load.
    await expect(page.getByTestId('container-usage-total')).toContainText('2 containers')
    await expect(page.getByTestId('container-usage-unavailable')).toContainText(
      '1 of 2 containers did not report usage',
    )

    // Scoped to what is on screen: an unscoped `docker stats` would measure a daemon this
    // computer shares with unrelated work.
    await expect.poll(async () => await sampledIds(page)).toEqual([[WEB, DB]])
  })

  test('a container under load reads higher than its idle siblings, by the newest tick', async ({
    page,
  }) => {
    await scene(page)
    await page.getByTestId('containers-group-shop').click()

    // Two ticks arrive; the panel ends up showing the second. A figure that stayed on the first
    // reading would be a live view that is not live.
    await expect(page.getByTestId(`container-usage-cpu-${WEB}`)).toHaveText(/CPU 4\d\.\d%/)
    await expect(page.getByTestId('container-usage')).toHaveAttribute('data-ticks', '2')
  })

  test('a container the runtime cannot measure reads unavailable, never zero', async ({ page }) => {
    await scene(page)
    await page.getByTestId('containers-group-shop').click()

    // Both figures, because a daemon that cannot read one container's cgroup cannot read either of
    // its numbers — and "0.0% / 0 B" would be a claim it never made (FR-038).
    await expect(page.getByTestId(`container-usage-cpu-${DB}`)).toHaveText('CPU unavailable')
    await expect(page.getByTestId(`container-usage-mem-${DB}`)).toHaveText(
      'Memory unavailable',
    )
  })

  test('the kept history states its own cap', async ({ page }) => {
    await scene(page)
    await page.getByTestId('containers-group-shop').click()

    // FR-038: a window that has started dropping its oldest readings says so, so a peak taken
    // over it is read for what it is.
    await expect(page.getByTestId('container-usage-at-cap')).toContainText('are being dropped')
  })

  test('leaving the section stops the sampling', async ({ page }) => {
    await scene(page)
    await page.getByTestId('containers-group-shop').click()
    await expect(page.getByTestId('container-usage')).toBeVisible()

    const nav = page.getByRole('navigation', { name: 'Sections' })
    if (!(await nav.isVisible())) {
      await page.getByRole('button', { name: 'Open sections' }).click()
    }
    await nav.locator('li button', { hasText: 'Pods' }).first().click()
    await expect(page.getByTestId('container-usage')).toBeHidden()

    // The panel unmounting is what stops it: nothing has to remember to (FR-045).
    await expect
      .poll(async () => (await sentTypes(page)).filter((t) => t === 'StopContainerUsageSampling'))
      .toEqual(['StopContainerUsageSampling'])
    // And nothing started it again on the way out.
    await expect.poll(async () => (await sampledIds(page)).length).toBe(1)
  })

  test('selecting one container samples only that one', async ({ page }) => {
    await scene(page)
    await page.getByTestId('containers-group-shop').click()
    await page.getByTestId('container-row-shop-web').click()

    await expect(page.getByTestId(`container-usage-row-${WEB}`)).toBeVisible()
    await expect(page.getByTestId(`container-usage-row-${DB}`)).toHaveCount(0)
    // A single container is its own figure, not an aggregate of one.
    await expect(page.getByTestId('container-usage-total')).toHaveCount(0)
    await expect
      .poll(async () => await sampledIds(page))
      .toEqual([[WEB, DB], [WEB]])
  })
})
