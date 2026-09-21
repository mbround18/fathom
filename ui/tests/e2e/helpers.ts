/**
 * Shared Playwright helpers for the US1 specs: drive the fake backend (`window.__fathomFake`)
 * and find panes and tables by their test hooks (contracts/layout.md § Test hooks).
 */
import { expect, type Locator, type Page, type TestInfo } from '@playwright/test'

import type { ScenarioName } from '../fixtures'
import type { BackendCmd, EventBatch } from '../../src/protocol/types'
import type {} from '../../src/protocol/fake-transport'

export const PODS = 'workloads/pods'
export const DEPLOYMENTS = 'workloads/deployments'

export function isPhone(info: TestInfo): boolean {
  return info.project.name.endsWith('-phone')
}

export async function open(page: Page, scenario: ScenarioName = 'default'): Promise<void> {
  await page.goto(`/?scenario=${scenario}`)
  await page.waitForFunction(() => window.__fathomFake !== undefined)
}

export async function emit(page: Page, batch: EventBatch): Promise<void> {
  await page.evaluate((b) => window.__fathomFake?.emit(b), batch)
}

export async function sent(page: Page): Promise<BackendCmd[]> {
  return page.evaluate(() => window.__fathomFake?.sent() ?? [])
}

export async function clearSent(page: Page): Promise<void> {
  await page.evaluate(() => window.__fathomFake?.clearSent())
}

/** Command `type`s sent since the last `clearSent`. */
export async function sentTypes(page: Page): Promise<string[]> {
  return (await sent(page)).map((c) => c.type)
}

/** The resource table showing `section` (the first, if several panes show it). */
export function table(page: Page, section: string = PODS): Locator {
  return page.locator(`[data-testid="resource-table"][data-section="${section}"]`).first()
}

/** The list pane host showing `section` (whatever state it is in). */
export function listPane(page: Page, section: string = PODS): Locator {
  return page
    .locator(`[data-slot="pane"][data-pane-kind="list"][data-section="${section}"]`)
    .first()
}

export function searchBox(page: Page, section: string = PODS): Locator {
  return listPane(page, section).getByRole('searchbox')
}

export function inspector(page: Page): Locator {
  return page.locator('[data-slot="pane"][data-pane-kind="inspector"]').first()
}

export function row(page: Page, key: string, section: string = PODS): Locator {
  return table(page, section).locator(`[role="row"][data-row-key="${key}"]`)
}

/** Wait until the table for `section` reports `count` rows (after filtering). */
export async function expectRowCount(page: Page, count: number, section = PODS): Promise<void> {
  await expect(table(page, section)).toHaveAttribute('data-row-count', String(count))
}

/** Open `section` from the navigator (the drawer on phone/tablet, the nav slot otherwise). */
export async function navigateTo(page: Page, label: string, group = 'Workloads'): Promise<void> {
  const nav = page.getByRole('navigation', { name: 'Sections' })
  if (!(await nav.isVisible())) {
    await page.getByRole('button', { name: 'Open navigator' }).click()
  }
  await expect(nav).toBeVisible()
  const toggle = nav.getByRole('button', { name: group, exact: true })
  if ((await toggle.getAttribute('aria-expanded')) === 'false') await toggle.click()
  await nav.getByRole('button', { name: label, exact: true }).click()
}

/** No horizontal page scroll (contracts/layout.md § Verification). */
export async function expectNoHorizontalScroll(page: Page): Promise<void> {
  const { scrollWidth, clientWidth } = await page.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: document.documentElement.clientWidth,
  }))
  expect(scrollWidth).toBeLessThanOrEqual(clientWidth)
}
