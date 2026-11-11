/**
 * Spec 063 US2, the grouped navigation (T031; FR-001 to FR-006). On desktop three headed groups in
 * order, Settings pinned last for owners and admins; on a phone one strip with a rule before groups
 * 2 and 3 and no visible headings; each group announced by name, the current link still current.
 * Also T011: the account menu's Help item.
 */
import type { Locator, Page } from '@playwright/test'

import { expect, fixtures, test, type FakeBackend } from '../../support/fake-backend'
import type { Me } from '../../../src/protocol/wire'

const GROUPS = [
  { id: 'pipelines', name: 'Pipelines', links: ['Runs', 'Reviews', 'Bindings', 'Receivers'] },
  { id: 'infrastructure', name: 'Infrastructure', links: ['Inventory', 'Hosts'] },
  { id: 'administration', name: 'Administration', links: ['Teams', 'Users', 'Audit', 'Migration'] },
] as const

async function open(page: Page, backend: FakeBackend, instance_role: Me['instance_role'], width: number) {
  backend.me = fixtures.me({ instance_role })
  await page.setViewportSize({ width, height: 800 })
  await page.goto('/runs')
  const nav = page.getByRole('navigation', { name: 'Sections' })
  await expect(nav.getByRole('link', { name: 'Runs' })).toHaveAttribute('aria-current', 'page')
  return nav
}

/** The nav's group headings and links, in document order. */
function entries(page: Page): Locator {
  return page.getByRole('navigation', { name: 'Sections' }).locator('[data-nav-group] > span, a')
}

const style = (l: Locator, prop: 'color' | 'borderLeftWidth' | 'marginTop') =>
  l.evaluate((el, p) => getComputedStyle(el)[p], prop)

test.describe('at 1280 px', () => {
  test('the headings come in order, each before its links, with Settings last for an owner', async ({ page, backend }) => {
    await open(page, backend, 'owner', 1280)
    await expect(entries(page)).toHaveText([...GROUPS.flatMap((g) => [g.name, ...g.links]), 'Settings'])
    for (const { id } of GROUPS) {
      const heading = await page.locator(`[data-nav-group="${id}"] > span`).boundingBox()
      expect(heading?.width ?? 0).toBeGreaterThan(1)
    }
  })

  test('a user has no Settings', async ({ page, backend }) => {
    await open(page, backend, 'user', 1280)
    await expect(entries(page)).toHaveText(GROUPS.flatMap((g) => [g.name, ...g.links]))
  })

  test('headings never take the accent', async ({ page, backend }) => {
    const nav = await open(page, backend, 'owner', 1280)
    const current = await style(nav.getByRole('link', { name: 'Runs' }), 'color')
    for (const { id } of GROUPS) {
      expect(await style(page.locator(`[data-nav-group="${id}"] > span`), 'color')).not.toBe(current)
    }
  })

  test('groups after the first sit lower, with no rule beside them', async ({ page, backend }) => {
    await open(page, backend, 'owner', 1280)
    for (const { id } of GROUPS) {
      expect(await style(page.locator(`[data-nav-group="${id}"]`), 'borderLeftWidth')).toBe('0px')
    }
    expect(parseFloat(await style(page.locator('[data-nav-group="infrastructure"]'), 'marginTop'))).toBeGreaterThan(0)
  })
})

test('at 360 px no heading shows, a rule comes before groups 2 and 3, and the page does not scroll sideways', async ({
  page,
  backend,
}) => {
  await open(page, backend, 'owner', 360)
  for (const [i, { id, links }] of GROUPS.entries()) {
    const group = page.locator(`[data-nav-group="${id}"]`)
    // sr-only: a 1px clipped box, read out but not seen.
    const heading = await group.locator('> span').boundingBox()
    expect(heading?.width ?? 0).toBeLessThanOrEqual(1)
    expect(await style(group, 'borderLeftWidth')).toBe(i === 0 ? '0px' : '1px')
    // The rule sits before the group's first link.
    if (i > 0) {
      const box = await group.boundingBox()
      const first = await group.getByRole('link', { name: links[0] }).boundingBox()
      expect(box?.x ?? 0).toBeLessThan(first?.x ?? 0)
    }
  }
  // Settings stays last in the strip.
  await expect(entries(page).last()).toHaveText('Settings')
  const width = await page.evaluate(() => document.documentElement.scrollWidth)
  expect(width).toBeLessThanOrEqual(360)
})

for (const width of [1280, 360]) {
  test(`each group is announced by name at ${String(width)} px, and the current link stays current`, async ({
    page,
    backend,
  }) => {
    await open(page, backend, 'user', width)
    for (const { name, links } of GROUPS) {
      const group = page.getByRole('group', { name })
      for (const link of links) await expect(group.getByRole('link', { name: link })).toHaveCount(1)
    }
    const hosts = page.getByRole('group', { name: 'Infrastructure' }).getByRole('link', { name: 'Hosts' })
    await hosts.click()
    await expect(hosts).toHaveAttribute('aria-current', 'page')
    await expect(page.locator('nav a[aria-current="page"]')).toHaveCount(1)
  })
}

test('the account menu’s Help item goes to /help', async ({ page, backend }) => {
  await open(page, backend, 'user', 1280)
  await page.getByRole('button', { name: 'Account' }).click()
  await page.getByRole('menuitem', { name: 'Help' }).click()
  await expect(page).toHaveURL(/\/help$/)
})
